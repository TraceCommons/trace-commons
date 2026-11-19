using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The window's feature flags, read as the Mac reads them
/// (macos/Sources/TCShellCore/FeatureFlags.swift,
/// TraceCommonsApp/AppModel+FeatureFlags.swift, DaemonClient+FeatureFlags.swift).
/// </summary>
public class FeatureFlagsTests
{
    private const string Digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    private static readonly BuildFeatureProfile Build = new(1, "testing", Digest);

    private static JsonObject Snapshot(bool? skills, ulong generation = 1, string instance = "6f1c1d3e-6a0b-4b8e-9a64-0d1f2e3c4b5a",
        string environment = "testing", string digest = Digest)
    {
        var flags = new JsonObject();
        if (skills is { } on)
        {
            flags["insights_skills"] = new JsonObject { ["enabled"] = on, ["pending"] = false, ["capability_blocked"] = false, ["timing"] = "live" };
        }

        return new JsonObject
        {
            ["schema_version"] = 1, ["environment"] = environment, ["profile_digest"] = digest, ["instance_id"] = instance,
            ["generation"] = generation, ["sampled_at"] = "2026-11-05T20:00:00Z", ["source"] = "default", ["fresh"] = false,
            ["flags"] = flags,
        };
    }

    private static FeatureFlagSnapshot? Parse(JsonObject o) => FeatureFlagSnapshot.Parse(JsonDocument.Parse(o.ToJsonString()).RootElement);

    [Fact]
    public void MissingFalseAndTrueReadAsTheMacReadsThem()
    {
        Assert.False(Parse(Snapshot(null))!.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.False(Parse(Snapshot(false))!.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.True(Parse(Snapshot(true))!.IsEnabled(FeatureFlagGate.InsightsSkills));
    }

    [Fact]
    public void ASnapshotFailingTheMacsChecksIsNone()
    {
        Assert.Null(Parse(Snapshot(true, generation: 0)));
        Assert.Null(Parse(Snapshot(true, instance: "not-a-uuid")));
        Assert.Null(Parse(Snapshot(true, environment: "dev")));
        Assert.Null(Parse(Snapshot(true, digest: Digest.ToUpperInvariant())));
        Assert.Null(Parse(Snapshot(true, digest: "abc")));
        JsonObject wrongVersion = Snapshot(true);
        wrongVersion["schema_version"] = 2;
        Assert.Null(Parse(wrongVersion));
        JsonObject badTiming = Snapshot(true);
        badTiming["flags"]!["insights_skills"]!["timing"] = "soon";
        Assert.Null(Parse(badTiming));
        JsonObject badSource = Snapshot(true);
        badSource["source"] = "guess";
        Assert.Null(Parse(badSource));
    }

    [Fact]
    public void ASuccessorIsTheSameBuildAndAnotherInstanceOrALaterGeneration()
    {
        FeatureFlagSnapshot first = Parse(Snapshot(true, generation: 2))!;
        Assert.True(first.AcceptsSuccessor(Parse(Snapshot(false, generation: 3))!));
        Assert.False(first.AcceptsSuccessor(Parse(Snapshot(false, generation: 2))!));
        Assert.False(first.AcceptsSuccessor(Parse(Snapshot(false, generation: 1))!));
        Assert.True(first.AcceptsSuccessor(Parse(Snapshot(false, generation: 1, instance: "00000000-0000-4000-8000-000000000001"))!));
        Assert.False(first.AcceptsSuccessor(Parse(Snapshot(false, generation: 9, environment: "staging"))!));
    }

    [Fact]
    public void TheBuildProfileAgreesOnlyWithItsOwnEnvironmentAndDigest()
    {
        Assert.True(Build.Agrees("testing", Digest));
        Assert.False(Build.Agrees("production", Digest));
        Assert.False(Build.Agrees("testing", Digest.Replace('a', 'b')));
        Assert.False(new BuildFeatureProfile(2, "testing", Digest).Agrees("testing", Digest));
        Assert.Null(BuildFeatureProfile.Parse("{\"environment\":\"testing\"}"));
        Assert.Null(BuildFeatureProfile.Parse("not json"));
    }

    private sealed class Daemon
    {
        public readonly Queue<string> Replies = new();
        public readonly List<string> Methods = new();

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Methods.Add(method);
            return Task.FromResult(DaemonResponse.Parse(Replies.Count > 0 ? Replies.Dequeue() : "{\"id\":0,\"result\":{\"requested\":true}}"));
        }

        public void Answer(JsonObject snapshot) => Replies.Enqueue(new JsonObject { ["id"] = 0, ["result"] = snapshot }.ToJsonString());
    }

    [Fact]
    public async Task TheGateTurnsSkillsOnOnlyForThisBuildsSnapshot()
    {
        var daemon = new Daemon();
        var gate = new FeatureFlagGate(daemon.Call, () => Build);
        Assert.False(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
        daemon.Answer(Snapshot(true));
        await gate.RefreshAsync();
        Assert.True(gate.IsEnabled(FeatureFlagGate.InsightsSkills));

        // Another build's snapshot is none: everything off.
        daemon.Answer(Snapshot(true, generation: 2, digest: new string('b', 64)));
        await gate.RefreshAsync();
        Assert.Null(gate.Current);

        // An error frame is none too.
        daemon.Answer(Snapshot(true, generation: 3));
        await gate.RefreshAsync();
        Assert.True(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
        daemon.Replies.Enqueue("{\"id\":0,\"error\":{\"code\":\"unavailable\",\"message\":\"x\"}}");
        await gate.RefreshAsync();
        Assert.False(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
    }

    [Fact]
    public async Task AnOlderGenerationFromTheSameInstanceNeverLandsOverANewerOne()
    {
        var daemon = new Daemon();
        var gate = new FeatureFlagGate(daemon.Call, () => Build);
        daemon.Answer(Snapshot(true, generation: 5));
        await gate.RefreshAsync();
        daemon.Answer(Snapshot(false, generation: 4));
        await gate.RefreshAsync();
        Assert.True(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.Equal(5UL, gate.Current!.Generation);
    }

    [Fact]
    public async Task ALateReadIsDroppedWhenANewerReadStarted()
    {
        var first = new TaskCompletionSource<DaemonResponse>();
        int calls = 0;
        var gate = new FeatureFlagGate((_, _) => ++calls == 1
            ? first.Task
            : Task.FromResult(DaemonResponse.Parse(new JsonObject { ["id"] = 0, ["result"] = Snapshot(false, generation: 2) }.ToJsonString())),
            () => Build);
        Task stale = gate.RefreshAsync();
        await gate.RefreshAsync();
        first.SetResult(DaemonResponse.Parse(new JsonObject { ["id"] = 0, ["result"] = Snapshot(true, generation: 9) }.ToJsonString()));
        await stale;
        Assert.False(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.Equal(2UL, gate.Current!.Generation);
    }

    [Fact]
    public async Task AWakeAsksTheDaemonToRefreshThenReads()
    {
        var daemon = new Daemon();
        var gate = new FeatureFlagGate(daemon.Call, () => Build);
        daemon.Replies.Enqueue("{\"id\":0,\"result\":{\"requested\":true}}");
        daemon.Answer(Snapshot(true));
        await gate.RefreshAfterWakeAsync();
        Assert.Equal(new[] { FeatureFlagGate.RefreshMethod, FeatureFlagGate.SnapshotMethod }, daemon.Methods);
        Assert.True(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
    }

    [Fact]
    public async Task WithNoDaemonEveryFlagIsOff()
    {
        var gate = new FeatureFlagGate(null, () => Build);
        await gate.RefreshAsync();
        await gate.RefreshAfterWakeAsync();
        Assert.Null(gate.Current);
        Assert.False(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
    }

    [Fact]
    public void TheLibraryNamesAValidBuildProfile()
    {
        BuildFeatureProfile? build = BuildFeatureProfile.FromLibrary();
        Assert.NotNull(build);
        Assert.True(build!.Valid, build.ToString());
    }

    /// <summary>
    /// Probe: a real daemon on a throwaway directory answers
    /// <c>feature_flags_snapshot</c> with a snapshot this parser accepts and
    /// that agrees with <c>tc_feature_profile_json</c>, and, with the flag
    /// off, <c>insights_skills</c> answers disabled with its words, which the
    /// Skills parser reads. With the flag on, the read would scan this
    /// machine's skill folders, so it is not made.
    /// </summary>
    [Fact]
    public async Task ARealDaemonsSnapshotAgreesWithThisBuild()
    {
        string dir = Path.Combine(Path.GetTempPath(), "tc-" + Guid.NewGuid().ToString("n").Substring(0, 8));
        Directory.CreateDirectory(Path.Combine(dir, "claude"));
        Directory.CreateDirectory(Path.Combine(dir, "codex"));
        string settings = JsonSerializer.Serialize(new { claude_root = Path.Combine(dir, "claude"), codex_root = Path.Combine(dir, "codex") });
        try
        {
            using var daemon = new TcDaemon(dir, settings);
            var gate = new FeatureFlagGate((method, parameters) => Task.FromResult(DaemonResponse.Parse(daemon.Call(method, parameters))));
            await gate.RefreshAsync();
            Assert.NotNull(gate.Current);
            Assert.True(BuildFeatureProfile.FromLibrary()!.Agrees(gate.Current!.Environment, gate.Current.ProfileDigest));
            Assert.True(gate.Current.Flags.ContainsKey(FeatureFlagGate.InsightsSkills), "the registry names insights_skills");
            if (!gate.IsEnabled(FeatureFlagGate.InsightsSkills))
            {
                DaemonResponse reply = DaemonResponse.Parse(daemon.Call(InsightsSkillsModel.Method, "{\"refresh\":false}"));
                SkillsSnapshot skills = SkillsSnapshot.Parse(reply.Result!.Value)!;
                Assert.False(skills.Enabled);
                Assert.Equal("Skills", skills.Copy["skills_title"]);
            }
        }
        finally
        {
            try { Directory.Delete(dir, true); } catch (IOException) { }
        }
    }
}
