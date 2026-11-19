using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The window's feature flags, read as the Mac reads them
/// (macos/Sources/TCShellCore/FeatureFlags.swift,
/// TraceCommonsApp/AppModel+FeatureFlags.swift, DaemonClient+FeatureFlags.swift).
/// </summary>
/// <remarks>
/// Where the decoding is not obvious, the expected answers are the Swift's
/// own: FeatureFlags.swift compiled alone with <c>swiftc</c> and asked each
/// case (2026-11-19). Integral numbers in any notation decode, a fractional
/// one does not; instance ids compare as exact strings, so the same UUID in
/// another case is another instance.
/// </remarks>
public class FeatureFlagsTests
{
    private const string Digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    private const string Instance = "6f1c1d3e-6a0b-4b8e-9a64-0d1f2e3c4b5a";
    private static readonly BuildFeatureProfile Build = new(1, "testing", Digest);

    private static JsonObject Snapshot(bool? skills, ulong generation = 1, string instance = Instance,
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

    private static FeatureFlagSnapshot? Parse(JsonObject o) => Parse(o.ToJsonString());

    private static FeatureFlagSnapshot? Parse(string json) => FeatureFlagSnapshot.Parse(JsonDocument.Parse(json).RootElement);

    [Fact]
    public void MissingFalseAndTrueReadAsTheMacReadsThem()
    {
        Assert.False(Parse(Snapshot(null))!.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.False(Parse(Snapshot(false))!.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.True(Parse(Snapshot(true))!.IsEnabled(FeatureFlagGate.InsightsSkills));
        Assert.False(Parse(Snapshot(true))!.IsEnabled("unknown"));
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
        JsonObject stringEnabled = Snapshot(true);
        stringEnabled["flags"]!["insights_skills"]!["enabled"] = "true";
        Assert.Null(Parse(stringEnabled));
        Assert.Null(Parse("{}"));
    }

    /// <summary>Each case's answer is the Swift probe's (see the class remarks).</summary>
    [Theory]
    [InlineData("schema 1.0", "\"schema_version\":1", "\"schema_version\":1.0", true)]
    [InlineData("schema 1e0", "\"schema_version\":1", "\"schema_version\":1e0", true)]
    [InlineData("schema 10e-1", "\"schema_version\":1", "\"schema_version\":10e-1", true)]
    [InlineData("schema 1.5", "\"schema_version\":1", "\"schema_version\":1.5", false)]
    [InlineData("generation 1.0", "\"generation\":1,", "\"generation\":1.0,", true)]
    [InlineData("generation 2e0", "\"generation\":1,", "\"generation\":2e0,", true)]
    [InlineData("generation 1.5", "\"generation\":1,", "\"generation\":1.5,", false)]
    [InlineData("generation -1", "\"generation\":1,", "\"generation\":-1,", false)]
    [InlineData("generation u64 max", "\"generation\":1,", "\"generation\":18446744073709551615,", true)]
    [InlineData("generation past u64", "\"generation\":1,", "\"generation\":18446744073709551616,", false)]
    [InlineData("uppercase instance", Instance, "6F1C1D3E-6A0B-4B8E-9A64-0D1F2E3C4B5A", true)]
    [InlineData("braced instance", "\"" + Instance + "\"", "\"{" + Instance + "}\"", false)]
    [InlineData("padded instance", "\"" + Instance + "\"", "\" " + Instance + "\"", false)]
    [InlineData("undashed instance", Instance, "6f1c1d3e6a0b4b8e9a640d1f2e3c4b5a", false)]
    [InlineData("enabled as 1", "\"enabled\":true", "\"enabled\":1", false)]
    [InlineData("fresh as 0", "\"fresh\":false", "\"fresh\":0", false)]
    [InlineData("pending missing", "\"pending\":false,", "", false)]
    [InlineData("next_start timing", "\"timing\":\"live\"", "\"timing\":\"next_start\"", true)]
    [InlineData("extra decision field", "\"timing\":\"live\"", "\"timing\":\"live\",\"z\":1", true)]
    [InlineData("extra snapshot field", "\"fresh\":false", "\"fresh\":false,\"extra\":1", true)]
    [InlineData("flags an array", "\"flags\":{", "\"flags\":[],\"x\":{", false)]
    [InlineData("no sampled_at", "\"sampled_at\":\"2026-11-05T20:00:00Z\",", "", false)]
    [InlineData("revision null", "\"fresh\":false", "\"fresh\":false,\"revision\":null", true)]
    [InlineData("revision a string", "\"fresh\":false", "\"fresh\":false,\"revision\":\"1\"", false)]
    [InlineData("expires_at null", "\"fresh\":false", "\"fresh\":false,\"expires_at\":null", true)]
    [InlineData("expires_at a number", "\"fresh\":false", "\"fresh\":false,\"expires_at\":5", false)]
    public void DecodingAgreesWithTheSwiftProbe(string name, string from, string to, bool decodes)
    {
        string json = Snapshot(true).ToJsonString();
        Assert.Contains(from, json, StringComparison.Ordinal);
        Assert.True(decodes == Parse(json.Replace(from, to, StringComparison.Ordinal)) is not null, name);
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
        Assert.False(first.AcceptsSuccessor(Parse(Snapshot(false, generation: 9, digest: new string('b', 64)))!));
    }

    /// <summary>
    /// Swift compares instance ids as strings (FeatureFlags.swift:68), so the
    /// same UUID in upper case is another instance and its generation 1 is
    /// accepted after generation 1. The probe answered true.
    /// </summary>
    [Fact]
    public void InstanceIdsCompareAsExactStringsAsInSwift()
    {
        FeatureFlagSnapshot lower = Parse(Snapshot(true))!;
        FeatureFlagSnapshot upper = Parse(Snapshot(false, instance: Instance.ToUpperInvariant()))!;
        Assert.True(lower.AcceptsSuccessor(upper));
    }

    [Fact]
    public void TheBuildProfileAgreesOnlyWithItsOwnEnvironmentAndDigest()
    {
        Assert.True(Build.Agrees("testing", Digest));
        Assert.False(Build.Agrees("production", Digest));
        Assert.False(Build.Agrees("testing", Digest.Replace('a', 'b')));
        Assert.False(Build.Agrees("testing", Digest.ToUpperInvariant()));
        Assert.False(Build.Agrees(null, Digest));
        Assert.False(Build.Agrees(null, null));
        Assert.False(new BuildFeatureProfile(2, "testing", Digest).Agrees("testing", Digest));
        Assert.Null(BuildFeatureProfile.Parse("{\"environment\":\"testing\"}"));
        Assert.Null(BuildFeatureProfile.Parse("not json"));
        Assert.Null(BuildFeatureProfile.Parse(null));

        // The probe: a profile of another schema or environment decodes, but
        // agrees with nothing.
        BuildFeatureProfile? v2 = BuildFeatureProfile.Parse("{\"schema_version\":2,\"environment\":\"staging\",\"profile_digest\":\"" + Digest + "\"}");
        Assert.NotNull(v2);
        Assert.False(v2!.Agrees("staging", Digest));
        BuildFeatureProfile? dev = BuildFeatureProfile.Parse("{\"schema_version\":1,\"environment\":\"dev\",\"profile_digest\":\"" + Digest + "\"}");
        Assert.NotNull(dev);
        Assert.False(dev!.Agrees("dev", Digest));
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
    public async Task TheGateTurnsAFlagOnOnlyForThisBuildsSnapshot()
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
    public async Task AnUnreadableBuildProfileIsNoSnapshot()
    {
        var daemon = new Daemon();
        var gate = new FeatureFlagGate(daemon.Call, () => null);
        daemon.Answer(Snapshot(true));
        await gate.RefreshAsync();
        Assert.Null(gate.Current);
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

        // A restarted daemon (another instance) starts over at a low generation.
        daemon.Answer(Snapshot(false, generation: 1, instance: "00000000-0000-4000-8000-000000000001"));
        await gate.RefreshAsync();
        Assert.False(gate.IsEnabled(FeatureFlagGate.InsightsSkills));
    }

    [Fact]
    public async Task ResetClearsAndDropsAReadAlreadyInFlight()
    {
        var pending = new TaskCompletionSource<DaemonResponse>();
        var gate = new FeatureFlagGate((_, _) => pending.Task, () => Build);
        int changes = 0;
        gate.Changed += () => changes++;
        Task read = gate.RefreshAsync();
        gate.Reset();
        pending.SetResult(DaemonResponse.Parse(new JsonObject { ["id"] = 0, ["result"] = Snapshot(true) }.ToJsonString()));
        await read;
        Assert.Null(gate.Current);
        Assert.Equal(0, changes);
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

    private static (string Dir, string Settings) ThrowawayDaemonDir()
    {
        string dir = Path.Combine(Path.GetTempPath(), "tc-" + Guid.NewGuid().ToString("n").Substring(0, 8));
        Directory.CreateDirectory(Path.Combine(dir, "claude"));
        Directory.CreateDirectory(Path.Combine(dir, "codex"));
        return (dir, JsonSerializer.Serialize(new { claude_root = Path.Combine(dir, "claude"), codex_root = Path.Combine(dir, "codex") }));
    }

    /// <summary>
    /// A real daemon on a throwaway directory answers
    /// <c>feature_flags_snapshot</c> with a snapshot this parser accepts and
    /// that agrees with <c>tc_feature_profile_json</c>.
    /// </summary>
    [Fact]
    public async Task ARealDaemonsSnapshotAgreesWithThisBuild()
    {
        (string dir, string settings) = ThrowawayDaemonDir();
        try
        {
            using var daemon = new TcDaemon(dir, settings);
            var gate = new FeatureFlagGate((method, parameters) => Task.FromResult(DaemonResponse.Parse(daemon.Call(method, parameters))));
            await gate.RefreshAsync();
            Assert.NotNull(gate.Current);
            Assert.True(BuildFeatureProfile.FromLibrary()!.Agrees(gate.Current!.Environment, gate.Current.ProfileDigest));
            Assert.True(gate.Current.Flags.ContainsKey(FeatureFlagGate.InsightsSkills), "the registry names insights_skills");
        }
        finally
        {
            try { Directory.Delete(dir, true); } catch (IOException) { }
        }
    }

    /// <summary>
    /// <c>feature_flags_changed</c> is opt-in (<c>OPT_IN_EVENTS</c>, ipc.rs):
    /// a subscription that declares it receives it, one that does not never
    /// does. Provoked with a local override, where this build allows one.
    /// </summary>
    [Fact]
    public void OnlyAnAcceptingSubscriptionHearsAFlagChange()
    {
        (string dir, string settings) = ThrowawayDaemonDir();
        try
        {
            using var daemon = new TcDaemon(dir, settings);
            JsonElement snapshot = DaemonResponse.Parse(daemon.Call(FeatureFlagGate.SnapshotMethod)).Result!.Value;
            string? overridable = null;
            if (snapshot.TryGetProperty("local_override_allowed", out JsonElement allowed) && allowed.ValueKind == JsonValueKind.Object)
            {
                foreach (JsonProperty p in allowed.EnumerateObject())
                {
                    if (p.Value.ValueKind == JsonValueKind.True)
                    {
                        overridable = p.Name;
                        break;
                    }
                }
            }

            Assert.True(overridable is not null, "this build allows no local override, so no change can be provoked");

            var plain = new ConcurrentQueue<string>();
            var accepting = new ConcurrentQueue<string>();
            using var heard = new ManualResetEventSlim(false);
            TcSubscription? plainSubscription = daemon.Subscribe(json => plain.Enqueue(json));
            TcSubscription? acceptingSubscription = daemon.Subscribe(
                json =>
                {
                    accepting.Enqueue(json);
                    if (DaemonEvent.Parse(json)?.Event == DaemonProtocol.Events.FeatureFlagsChanged) heard.Set();
                },
                new[] { DaemonProtocol.Events.FeatureFlagsChanged });
            Assert.NotNull(plainSubscription);
            Assert.NotNull(acceptingSubscription);

            string parameters = new JsonObject
            {
                ["id"] = overridable,
                ["value"] = !(snapshot.GetProperty("flags").GetProperty(overridable!).GetProperty("enabled").GetBoolean()),
                ["environment"] = snapshot.GetProperty("environment").GetString(),
                ["profile_digest"] = snapshot.GetProperty("profile_digest").GetString(),
            }.ToJsonString();
            DaemonResponse reply = DaemonResponse.Parse(daemon.Call("feature_flags_override_set", parameters));
            Assert.False(reply.IsError, reply.Error?.Message);

            Assert.True(heard.Wait(TimeSpan.FromSeconds(10)), "the accepting subscription never heard feature_flags_changed");
            Assert.True(daemon.Unsubscribe(acceptingSubscription!));
            Assert.True(daemon.Unsubscribe(plainSubscription!));
            Assert.DoesNotContain(plain, json => DaemonEvent.Parse(json)?.Event == DaemonProtocol.Events.FeatureFlagsChanged);
        }
        finally
        {
            try { Directory.Delete(dir, true); } catch (IOException) { }
        }
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }

    /// <summary>
    /// The App half, which only CI compiles: the host declares the opt-in
    /// event and raises it, and a lag or resync rereads the flags as on the
    /// Mac (AppModel.swift:1993); the window reads at every start, on the
    /// event, and after a wake (with a refresh request first).
    /// </summary>
    [Fact]
    public void TheWindowReadsFlagsWhereTheMacDoes()
    {
        string host = ShellSource("TraceCommons.App/DaemonHost.cs");
        Assert.Contains("daemon.Subscribe(OnDaemonEvent, new[] { DaemonProtocol.Events.FeatureFlagsChanged })", host, StringComparison.Ordinal);
        Assert.Contains("case DaemonProtocol.Events.FeatureFlagsChanged:\n                    FeatureFlagsChanged?.Invoke();", host, StringComparison.Ordinal);
        int lagged = host.IndexOf("case DaemonProtocol.Events.Lagged:", StringComparison.Ordinal);
        int resync = host.IndexOf("case DaemonProtocol.Events.ResyncRequired:", StringComparison.Ordinal);
        int preview = host.IndexOf("case DaemonProtocol.Events.PreviewReady:", StringComparison.Ordinal);
        Assert.True(lagged > 0 && resync > lagged && preview > resync);
        Assert.Contains("FeatureFlagsChanged?.Invoke();", host.Substring(lagged, resync - lagged), StringComparison.Ordinal);
        Assert.Contains("FeatureFlagsChanged?.Invoke();", host.Substring(resync, preview - resync), StringComparison.Ordinal);
        Assert.Contains("Started?.Invoke();", host, StringComparison.Ordinal);

        string window = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains("_host.Started += () => { _flags.Reset(); _ = _flags.RefreshAsync(); };", window, StringComparison.Ordinal);
        Assert.Contains("_host.FeatureFlagsChanged += () => _ = _flags.RefreshAsync();", window, StringComparison.Ordinal);
        Assert.Contains("WindowMinimumSize.Resumed += () => _host.Dispatcher.TryEnqueue(() => _ = _flags.RefreshAfterWakeAsync());", window, StringComparison.Ordinal);

        string shell = ShellSource("TraceCommons.App/MainWindow.Shell.cs");
        Assert.Contains("msg == WM_POWERBROADCAST && (int)wParam == PBT_APMRESUMEAUTOMATIC", shell, StringComparison.Ordinal);
    }
}
