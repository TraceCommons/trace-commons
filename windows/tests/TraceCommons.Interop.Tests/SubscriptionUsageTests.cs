using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Subscription usage (Claude and Codex limits) and the login cards that draw
/// it, ported from macos/Tests/TCShellCoreTests/SubscriptionUsageWireTests.swift,
/// macos/Tests/TraceCommonsAppTests/SubscriptionUsageTests.swift,
/// InsightsLimitsTableTests.swift and InsightsCreditTests.swift:62-75.
/// </summary>
public sealed class SubscriptionUsageTests
{
    private static readonly DateTimeOffset Now = DateTimeOffset.FromUnixTimeSeconds(1_792_000_000);

    private static readonly Lazy<Dictionary<string, string>> CoreCopy = new(() =>
    {
        var copy = new LocalInsights(Path.Combine(Path.GetTempPath(), "insights-copy-" + Guid.NewGuid().ToString("N")))
            .CallAsync(new { type = "copy" }, CancellationToken.None).GetAwaiter().GetResult();
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in copy.GetProperty("copy").EnumerateObject()) words[pair.Name] = pair.Value.GetString()!;
        return words;
    });

    private static string? Copy(string key) => CoreCopy.Value.TryGetValue(key, out string? word) ? word : null;

    private static SubscriptionUsage Parse(string json) => SubscriptionUsage.Parse(JsonDocument.Parse(json).RootElement)!;

    private static SubscriptionUsageWindow Window(string json) =>
        SubscriptionUsage.ParseWindow(JsonDocument.Parse(json).RootElement)!;

    private static SubscriptionUsageLogin Login(string json) =>
        SubscriptionUsage.ParseLogin(JsonDocument.Parse(json).RootElement)!;

    // ---- The wire (SubscriptionUsageWireTests.swift) ------------------------

    [Fact]
    public void PartialUsageKeepsUnknownsAndProviderWindowsSeparate()
    {
        SubscriptionUsage result = Parse("""{"schema_version":"trace_commons.subscription_usage.v1","providers":[{"provider":"claude","enabled":true,"refreshing":false,"logins":[{"id":"claude-default","label":"Claude","default_login":true,"state":"observed","reason":null,"updated_at":"2026-10-26T10:00:00.123Z","stale":false,"banked_resets":null,"windows":[{"lane":"session","label":null,"used_permille":null,"remaining_text":"—","resets_at":null},{"lane":"weekly","label":null,"used_permille":300,"remaining_text":"70%","resets_at":"2026-10-29T10:00:00Z"},{"lane":"model","label":"Fable","used_permille":800,"remaining_text":"20%","resets_at":"2026-10-29T10:00:00Z"}]}]}]}""");
        SubscriptionUsageLogin login = result.Providers[0].Logins[0];
        Assert.NotNull(login.UpdatedAt);
        Assert.Null(login.BankedResets);
        Assert.Null(login.Refreshing);
        Assert.Null(login.Windows[0].UsedPermille);
        Assert.Null(login.Windows[0].ResetsAt);
        Assert.Equal(300, login.Windows[1].UsedPermille);
        Assert.Equal(DateTimeOffset.Parse("2026-10-29T10:00:00Z"), login.Windows[1].ResetsAt);
        Assert.Equal("Fable", login.Windows[2].Label);
    }

    [Fact]
    public void PerLoginRefreshingIsOptionalAndIndependent()
    {
        SubscriptionUsage usage = Parse("""{"schema_version":"trace_commons.subscription_usage.v1","providers":[{"provider":"claude","enabled":true,"refreshing":true,"logins":[{"id":"one","label":"One","default_login":true,"state":"unknown","stale":false,"windows":[],"refreshing":true},{"id":"two","label":"Two","default_login":false,"state":"unknown","stale":false,"windows":[],"refreshing":false}]}]}""");
        Assert.Equal(new bool?[] { true, false }, usage.Providers[0].Logins.Select(l => l.Refreshing));
        Assert.True(usage.Providers[0].Refreshing);
    }

    [Fact]
    public void DurationIsOptionalPositiveAndPreservesNonstandardWindows()
    {
        Assert.Null(Window("""{"lane":"weekly"}""").DurationMins);
        foreach (int minutes in new[] { 540, 1_440 })
        {
            Assert.Equal(minutes, Window("{\"lane\":\"secondary\",\"duration_mins\":" + minutes + "}").DurationMins);
        }

        foreach (string invalid in new[] { "0", "-1", "null", "true", "\"540\"", "18446744073709551615", "1.5" })
        {
            Assert.Null(Window("{\"lane\":\"secondary\",\"duration_mins\":" + invalid + "}").DurationMins);
        }
    }

    [Fact]
    public void ReadingSourceIsOptionalAndToleratesOlderDaemons()
    {
        Assert.Equal("cli", Login("""{"id":"claude-default","label":"Claude","default_login":true,"state":"observed","stale":false,"windows":[],"source":"cli"}""").Source);
        Assert.Null(Login("""{"id":"claude-default","label":"Claude","default_login":true,"state":"unknown","stale":false,"windows":[],"source":null}""").Source);
        Assert.Null(Login("""{"id":"claude-default","label":"Claude","default_login":true,"state":"unknown","stale":false,"windows":[]}""").Source);
        Assert.Null(Login("""{"id":"claude-default","label":"Claude","default_login":true,"state":"observed","stale":false,"windows":[],"source":7}""").Source);
    }

    [Fact]
    public void InvalidOptionalMeasurementDoesNotDiscardValidNeighbors()
    {
        foreach (string invalid in new[] { "true", "\"20\"", "-1", "1001", "null" })
        {
            SubscriptionUsageWindow window = Window("{\"lane\":\"weekly\",\"label\":null,\"used_permille\":" + invalid
                + ",\"remaining_text\":\"do not infer\",\"resets_at\":\"invalid\"}");
            Assert.Null(window.UsedPermille);
            Assert.Null(window.ResetsAt);
            Assert.Equal("weekly", window.Lane);
        }

        Assert.Null(Login("""{"id":"codex-default","label":"Codex","default_login":true,"state":"observed","stale":false,"windows":[],"banked_resets":-1}""").BankedResets);
        Assert.Equal(0, Login("""{"id":"codex-default","label":"Codex","default_login":true,"state":"observed","stale":false,"windows":[],"banked_resets":0}""").BankedResets);
    }

    /// <summary>A required field missing or mistyped refuses the reply, as Swift's synthesized decode does.</summary>
    [Fact]
    public void ARequiredFieldMissingOrMistypedRefusesTheReply()
    {
        const string login = """{"id":"a","label":"A","default_login":true,"state":"unknown","stale":false,"windows":[]}""";
        Assert.NotNull(SubscriptionUsage.Parse(JsonDocument.Parse("{\"schema_version\":\"v\",\"providers\":[{\"provider\":\"codex\",\"enabled\":true,\"refreshing\":false,\"logins\":[" + login + "]}]}").RootElement));
        foreach (string broken in new[]
        {
            """{"providers":[]}""",
            """{"schema_version":"v"}""",
            """{"schema_version":"v","providers":[{"provider":"codex","refreshing":false,"logins":[]}]}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"logins":[]}]}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"refreshing":false}]}""",
            """{"schema_version":"v","providers":[],"truncated":"yes"}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"refreshing":false,"logins":[{"id":"a","label":"A","state":"unknown","stale":false,"windows":[]}]}]}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"refreshing":false,"logins":[{"id":"a","label":"A","default_login":true,"state":"unknown","windows":[]}]}]}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"refreshing":false,"logins":[{"id":"a","label":"A","default_login":true,"state":"unknown","stale":false,"windows":[],"reason":3}]}]}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"refreshing":false,"logins":[{"id":"a","label":"A","default_login":true,"state":"unknown","stale":false,"windows":[{"label":"x"}]}]}]}""",
            """{"schema_version":"v","providers":[{"provider":"codex","enabled":true,"refreshing":false,"logins":[{"id":"a","label":"A","default_login":true,"state":"unknown","stale":false,"windows":[{"lane":"weekly","label":4}]}]}]}""",
            "[]",
        })
        {
            Assert.Null(SubscriptionUsage.Parse(JsonDocument.Parse(broken).RootElement));
        }
    }

    /// <summary>
    /// A real reply from the daemon's own serializer
    /// (<c>subscription_usage::State::value</c>), pinned by the contributor
    /// crate's test <c>the_windows_subscription_usage_fixture_is_the_daemons_reply</c>.
    /// </summary>
    [Fact]
    public void TheDaemonsOwnReplyReads()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "SubscriptionUsage", "subscription_usage.json");
        SubscriptionUsage usage = Parse(File.ReadAllText(path));
        Assert.Equal(SubscriptionUsage.Schema, usage.SchemaVersion);
        Assert.Equal(new[] { "codex", "claude" }, usage.Providers.Select(p => p.Provider));
        SubscriptionUsageLogin codex = Assert.Single(usage.Providers[0].Logins);
        Assert.Equal("observed", codex.State);
        Assert.Equal(2, codex.BankedResets);
        Assert.Equal("api", codex.Source);
        SubscriptionUsageWindow weekly = Assert.Single(codex.Windows);
        Assert.Equal(420, weekly.UsedPermille);
        Assert.Equal(10_080, weekly.DurationMins);
        Assert.Equal("58%", weekly.RemainingText);
        Assert.NotNull(weekly.ResetsAt);
        SubscriptionUsageLogin claude = Assert.Single(usage.Providers[1].Logins);
        Assert.Equal("unknown", claude.State);
        Assert.Empty(claude.Windows);
        Assert.Null(claude.Source);
    }

    // ---- The call (LiveDaemonClient.swift:344-371) ---------------------------

    [Fact]
    public async Task TheClientSendsOnlyBoundedUsageAndProviderSettingCalls()
    {
        var calls = new List<(string Method, string Params)>();
        Task<DaemonResponse> Call(string method, string json)
        {
            calls.Add((method, json));
            return Task.FromResult(DaemonResponse.Parse(method == SubscriptionUsage.Method
                ? """{"id":1,"result":{"schema_version":"trace_commons.subscription_usage.v1","providers":[]}}"""
                : """{"id":1,"result":{}}"""));
        }

        SubscriptionUsageModel.Reader read = SubscriptionUsageClient.Reader(Call);
        SubscriptionUsageModel.Writer write = SubscriptionUsageClient.Writer(Call);
        Assert.Empty((await read(false, true, null, null)).Providers);
        await read(true, true, "codex", null);
        await write("codex", true);
        await write("claude", false);
        Assert.Equal(new[] { "subscription_usage", "subscription_usage", "set_settings", "set_settings" }, calls.Select(c => c.Method));
        Assert.Equal("""{"refresh":false,"interactive":false}""", calls[0].Params);
        Assert.Equal("""{"refresh":true,"interactive":true,"provider":"codex"}""", calls[1].Params);
        Assert.Equal("""{"subscription_usage_codex_enabled":true}""", calls[2].Params);
        Assert.Equal("""{"subscription_usage_claude_on":false}""", calls[3].Params);
        var refused = await Assert.ThrowsAsync<SubscriptionUsageException>(() => write("unrecognized", true));
        Assert.Equal("subscription-provider-invalid", refused.Message);
        refused = await Assert.ThrowsAsync<SubscriptionUsageException>(() => read(true, true, "unrecognized", null));
        Assert.Equal("subscription-provider-invalid", refused.Message);
        Assert.Equal(4, calls.Count);
    }

    [Fact]
    public async Task ClaudeRefreshSendsTheExactLoginAndRefusesMissingOrUnboundedSelectors()
    {
        var calls = new List<string>();
        Task<DaemonResponse> Call(string method, string json)
        {
            calls.Add(json);
            return Task.FromResult(DaemonResponse.Parse("""{"id":1,"result":{"schema_version":"trace_commons.subscription_usage.v1","providers":[]}}"""));
        }

        SubscriptionUsageModel.Reader read = SubscriptionUsageClient.Reader(Call);
        await read(true, true, "claude", "claude-profile-123");
        Assert.Equal("""{"refresh":true,"interactive":true,"provider":"claude","login":"claude-profile-123"}""", Assert.Single(calls));
        foreach (string? login in new[] { null, "", "line\nfeed", new string('x', 257) })
        {
            var refused = await Assert.ThrowsAsync<SubscriptionUsageException>(() => read(true, true, "claude", login));
            Assert.Equal("subscription-login-invalid", refused.Message);
        }

        // A login with no provider names nothing to read.
        await Assert.ThrowsAsync<SubscriptionUsageException>(() => read(false, false, null, "claude-profile-123"));
        Assert.Single(calls);
    }

    [Fact]
    public async Task TheClientNamesAnOlderDaemonAndAFailedOrMalformedRead()
    {
        SubscriptionUsageModel.Reader Reader(string reply) =>
            SubscriptionUsageClient.Reader((_, _) => Task.FromResult(DaemonResponse.Parse(reply)));
        var old = await Assert.ThrowsAsync<SubscriptionUsageException>(() =>
            Reader("""{"id":1,"error":{"code":"unknown_method","message":"unknown-method"}}""")(false, false, null, null));
        Assert.True(old.NotServed);
        var failed = await Assert.ThrowsAsync<SubscriptionUsageException>(() =>
            Reader("""{"id":1,"error":{"code":"internal","message":"x"}}""")(false, false, null, null));
        Assert.False(failed.NotServed);
        var malformed = await Assert.ThrowsAsync<SubscriptionUsageException>(() =>
            Reader("""{"id":1,"result":{"providers":[]}}""")(false, false, null, null));
        Assert.False(malformed.NotServed);
    }

    // ---- The model (SubscriptionUsageTests.swift:28-426) --------------------

    private static SubscriptionUsage Snapshot(bool enabled = true, bool refreshing = false) => new(new[]
    {
        new SubscriptionUsageProvider("codex", enabled, refreshing, enabled
            ? new[]
            {
                new SubscriptionUsageLogin("codex-default", "Codex", "observed", DefaultLogin: true, BankedResets: 2, Windows: new[]
                {
                    new SubscriptionUsageWindow("weekly", UsedPermille: 300, RemainingText: "70%", ResetsAt: Now.AddHours(1)),
                }),
            }
            : Array.Empty<SubscriptionUsageLogin>()),
    });

    private static SubscriptionUsage ClaudeSnapshot(bool codexEnabled = false, bool refreshing = false) => new(new[]
    {
        new SubscriptionUsageProvider("codex", codexEnabled),
        new SubscriptionUsageProvider("claude", true, refreshing, new[]
        {
            new SubscriptionUsageLogin("claude-default", "Default", "unknown", DefaultLogin: true, Reason: "not_checked", Refreshing: refreshing),
            new SubscriptionUsageLogin("claude-work", "Work", "unknown", Reason: "not_checked", Refreshing: false),
        }),
    });

    /// <summary>The Mac's <c>UsageReaderGate</c>: each read waits until the test answers it.</summary>
    private sealed class Gate
    {
        private readonly object _lock = new();
        private readonly List<(bool Refresh, bool Interactive, string? Provider, string? Login, TaskCompletionSource<SubscriptionUsage> Answer)> _calls = new();
        private readonly SubscriptionUsage? _initial;

        public Gate(SubscriptionUsage? initial = null) => _initial = initial;

        public Task<SubscriptionUsage> Read(bool refresh, bool interactive, string? provider, string? login)
        {
            var answer = new TaskCompletionSource<SubscriptionUsage>(TaskCreationOptions.RunContinuationsAsynchronously);
            lock (_lock)
            {
                // The first read answers at once with the initial snapshot, as the Mac's gate does.
                if (_initial is not null && _calls.Count == 0) answer.SetResult(_initial);
                _calls.Add((refresh, interactive, provider, login, answer));
            }

            return answer.Task;
        }

        public int Count { get { lock (_lock) return _calls.Count; } }

        public (bool, bool) Flags(int index) { lock (_lock) return (_calls[index].Refresh, _calls[index].Interactive); }

        public (string?, string?) Scope(int index) { lock (_lock) return (_calls[index].Provider, _calls[index].Login); }

        public void Resolve(int index, SubscriptionUsage value) { lock (_lock) _calls[index].Answer.TrySetResult(value); }

        public void Fail(int index, Exception error) { lock (_lock) _calls[index].Answer.TrySetException(error); }

        public void ResolveRest(int from, SubscriptionUsage value)
        {
            lock (_lock)
            {
                for (int i = from; i < _calls.Count; i++) _calls[i].Answer.TrySetResult(value);
            }
        }

        public async Task<bool> WaitForCalls(int count)
        {
            var clock = Stopwatch.StartNew();
            while (Count < count)
            {
                if (clock.Elapsed > TimeSpan.FromSeconds(5)) return false;
                await Task.Delay(1);
            }

            return true;
        }
    }

    private static async Task Settle(Func<bool> done, int milliseconds = 2_000)
    {
        var clock = Stopwatch.StartNew();
        while (!done() && clock.ElapsedMilliseconds < milliseconds) await Task.Delay(1);
    }

    private static Task Quiet() => Task.Delay(100);

    [Fact]
    public async Task NoReaderAndAnOlderDaemonNeverSubstituteUsage()
    {
        var absent = new SubscriptionUsageModel();
        await absent.LoadAsync(refresh: true, interactive: true);
        Assert.False(absent.IsVisible);
        var old = new SubscriptionUsageModel((_, _, _, _) => throw new SubscriptionUsageException("unknown-method", notServed: true));
        await old.LoadAsync();
        Assert.False(old.IsVisible);
        Assert.Null(old.Shown);
        Assert.Null(old.FailureKey);
        Assert.True(old.Unsupported);
    }

    [Fact]
    public async Task RefreshesCoalesceAndOnlyAnExplicitRefreshIsInteractive()
    {
        var gate = new Gate();
        var model = new SubscriptionUsageModel(gate.Read);
        Task first = model.LoadAsync();
        Assert.True(await gate.WaitForCalls(1));
        await model.LoadAsync(refresh: true, interactive: true);
        await model.LoadAsync(refresh: true, interactive: true);
        Assert.Equal(1, gate.Count);
        gate.Resolve(0, Snapshot());
        await first;
        Assert.True(await gate.WaitForCalls(2));
        Assert.Equal((false, false), gate.Flags(0));
        Assert.Equal((true, true), gate.Flags(1));
        gate.Resolve(1, Snapshot());
        await Settle(() => !model.IsRefreshing);
        Assert.Equal(2, gate.Count);
    }

    [Fact]
    public async Task StoppingHidesUsageAndDiscardsALateRead()
    {
        var gate = new Gate(Snapshot());
        var model = new SubscriptionUsageModel(gate.Read, (_, _) => Task.CompletedTask);
        await model.LoadAsync();
        Task old = model.LoadAsync();
        Assert.True(await gate.WaitForCalls(2));
        Task stop = model.SetEnabledAsync("codex", false);
        Assert.True(await gate.WaitForCalls(3));
        Assert.False(model.Shown!.Providers[0].Enabled);
        Assert.Empty(model.Shown!.Providers[0].Logins);
        gate.Resolve(2, Snapshot(enabled: false));
        await stop;
        gate.Resolve(1, Snapshot());
        await old;
        Assert.False(model.Shown!.Providers[0].Enabled);
        Assert.Empty(model.Shown!.Providers[0].Logins);
        Assert.Equal((false, false), gate.Flags(2));
    }

    [Fact]
    public async Task APermissionOnlySavesAndReadsWithoutAProbe()
    {
        var log = new ConcurrentQueue<string>();
        var model = new SubscriptionUsageModel(
            (refresh, interactive, provider, _) =>
            {
                log.Enqueue($"read:{refresh}:{interactive}:{provider ?? "all"}");
                return Task.FromResult(Snapshot());
            },
            (provider, on) =>
            {
                log.Enqueue($"set:{provider}:{on}");
                return Task.CompletedTask;
            });
        await model.SetEnabledAsync("codex", true);
        Assert.Equal(new[] { "set:codex:True", "read:False:False:all" }, log);
        Assert.True(model.IsVisible);
    }

    [Fact]
    public async Task ClaudeStartsOnAndCodexOffDoesNotHideIt()
    {
        var log = new ConcurrentQueue<string>();
        var model = new SubscriptionUsageModel(
            (refresh, interactive, provider, login) =>
            {
                log.Enqueue($"read:{refresh}:{interactive}:{provider ?? "all"}:{login ?? "all"}");
                return Task.FromResult(ClaudeSnapshot());
            },
            (provider, on) =>
            {
                log.Enqueue($"set:{provider}:{on}");
                return Task.CompletedTask;
            });
        await model.LoadAsync();
        Assert.True(model.IsVisible);
        Assert.False(model.CodexEnabled);
        Assert.Equal(new[] { "claude" }, model.SupportedProviders.Select(p => p.Provider));
        Assert.Equal(new[] { "claude-default", "claude-work" }, model.SupportedProviders[0].Logins.Select(l => l.Id));
        Assert.True(model.ClaudeEnabled);
        Assert.Equal(new[] { "read:False:False:all:all" }, log);
        Assert.False(model.CanRefresh("claude", null));
        Assert.False(model.CanRefresh("claude", "not-discovered"));
        Assert.True(model.CanRefresh("claude", "claude-work"));
        await model.SetEnabledAsync("codex", false);
        Assert.True(model.IsVisible);
        Assert.Equal(new[] { "claude" }, model.SupportedProviders.Select(p => p.Provider));
    }

    [Fact]
    public async Task ClaudeOffSavesAndHidesClaudeWithoutAProbe()
    {
        var log = new ConcurrentQueue<string>();
        var model = new SubscriptionUsageModel(
            (refresh, interactive, provider, login) =>
            {
                log.Enqueue($"read:{refresh}:{interactive}:{provider ?? "all"}:{login ?? "all"}");
                return Task.FromResult(ClaudeSnapshot());
            },
            (provider, on) =>
            {
                log.Enqueue($"set:{provider}:{on}");
                return Task.CompletedTask;
            });
        await model.LoadAsync();
        await model.SetEnabledAsync("claude", false);
        Assert.Equal("read:False:False:all:all", log.First());
        Assert.Contains("set:claude:False", log);
        Assert.DoesNotContain(log, line => line.StartsWith("read:True", StringComparison.Ordinal));
    }

    [Fact]
    public async Task ADisabledClaudeIsNotShownEvenWithLogins()
    {
        var model = new SubscriptionUsageModel((_, _, _, _) => Task.FromResult(new SubscriptionUsage(new[]
        {
            new SubscriptionUsageProvider("codex", false),
            new SubscriptionUsageProvider("claude", false, Logins: new[] { new SubscriptionUsageLogin("claude-default", "Default", "unknown", DefaultLogin: true) }),
        })), (_, _) => Task.CompletedTask);
        await model.LoadAsync();
        Assert.False(model.ClaudeEnabled);
        Assert.False(model.IsVisible);
        Assert.Empty(model.SupportedProviders);
    }

    [Fact]
    public async Task NoClaudeLoginsExposesNoEmptyProvider()
    {
        var model = new SubscriptionUsageModel((_, _, _, _) => Task.FromResult(new SubscriptionUsage(new[]
        {
            new SubscriptionUsageProvider("codex", false),
            new SubscriptionUsageProvider("claude", true),
        })));
        await model.LoadAsync();
        Assert.False(model.IsVisible);
        Assert.Empty(model.SupportedProviders);
    }

    [Fact]
    public async Task AClaudeRefreshTargetsOnlyItsLoginAndTheDefaultRefreshIsCodex()
    {
        var gate = new Gate(ClaudeSnapshot(codexEnabled: true));
        var model = new SubscriptionUsageModel(gate.Read);
        await model.LoadAsync();
        _ = model.Refresh("claude", "claude-work");
        Assert.True(await gate.WaitForCalls(2));
        Assert.Equal(("claude", "claude-work"), gate.Scope(1));
        Assert.True(model.IsRefreshingLogin("claude", "claude-work"));
        Assert.False(model.IsRefreshingLogin("claude", "claude-default"));
        Assert.False(model.IsRefreshingLogin("codex", null));
        gate.Resolve(1, ClaudeSnapshot(codexEnabled: true));
        await Settle(() => !model.IsRefreshing);
        _ = model.Refresh();
        Assert.True(await gate.WaitForCalls(3));
        Assert.Equal(("codex", null), gate.Scope(2));
        gate.Resolve(2, ClaudeSnapshot(codexEnabled: true));
    }

    [Fact]
    public async Task AQueuedLoginRefreshNeverWidensIntoOtherLoginReads()
    {
        var gate = new Gate();
        var model = new SubscriptionUsageModel(gate.Read);
        Task first = model.LoadAsync();
        Assert.True(await gate.WaitForCalls(1));
        await model.LoadAsync(refresh: true, interactive: true, provider: "claude", login: "claude-default");
        await model.LoadAsync(refresh: true, interactive: false, provider: "claude", login: "claude-work");
        gate.Resolve(0, ClaudeSnapshot());
        await first;
        Assert.True(await gate.WaitForCalls(2));
        Assert.Equal(("claude", "claude-work"), gate.Scope(1));
        Assert.Equal((true, false), gate.Flags(1));
        gate.Resolve(1, ClaudeSnapshot());
    }

    [Fact]
    public async Task APendingClaudeReadWakesTheObserverAndKeepsOnlyItsSpinner()
    {
        var gate = new Gate(ClaudeSnapshot());
        var model = new SubscriptionUsageModel(gate.Read, activePollInterval: TimeSpan.FromMilliseconds(1));
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(false, cancel.Token);
        Assert.True(await gate.WaitForCalls(1));
        await Quiet();
        _ = model.Refresh("claude", "claude-default");
        Assert.True(await gate.WaitForCalls(2));
        gate.Resolve(1, ClaudeSnapshot(refreshing: true));
        Assert.True(await gate.WaitForCalls(3));
        Assert.True(model.IsRefreshingLogin("claude", "claude-default"));
        Assert.False(model.IsRefreshingLogin("claude", "claude-work"));
        Assert.False(model.CanRefresh("claude", "claude-work"));
        Assert.Equal((false, false), gate.Flags(2));
        Assert.Equal((null, null), gate.Scope(2));
        cancel.Cancel();
        gate.ResolveRest(2, ClaudeSnapshot());
        await observing;
    }

    /// <summary>Opening Insights reads each unchecked login silently, once, and the next poll does not repeat it.</summary>
    [Fact]
    public async Task OpeningInsightsReadsUncheckedLoginsSilentlyOncePerLogin()
    {
        var gate = new Gate(ClaudeSnapshot());
        var model = new SubscriptionUsageModel(gate.Read, activePollInterval: TimeSpan.FromMilliseconds(1));
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(true, cancel.Token);
        Assert.True(await gate.WaitForCalls(2));
        Assert.Equal((true, false), gate.Flags(1));
        Assert.Equal(("claude", "claude-default"), gate.Scope(1));
        gate.Resolve(1, ClaudeSnapshot());
        Assert.True(await gate.WaitForCalls(3));
        Assert.Equal((true, false), gate.Flags(2));
        Assert.Equal(("claude", "claude-work"), gate.Scope(2));
        gate.Resolve(2, ClaudeSnapshot());
        await Quiet();
        int count = gate.Count;
        gate.ResolveRest(3, ClaudeSnapshot());
        Assert.Equal(3, count);
        cancel.Cancel();
        await observing;
    }

    [Fact]
    public async Task OpeningInsightsDoesNotReadLoginsThatHaveCurrentFigures()
    {
        var gate = new Gate(Snapshot());
        var model = new SubscriptionUsageModel(gate.Read);
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(true, cancel.Token);
        Assert.True(await gate.WaitForCalls(1));
        await Quiet();
        int count = gate.Count;
        gate.ResolveRest(1, Snapshot());
        Assert.Equal(1, count);
        cancel.Cancel();
        await observing;
    }

    [Fact]
    public async Task AllOffShowsNothingAndRefreshesNothing()
    {
        var model = new SubscriptionUsageModel((_, _, _, _) => Task.FromResult(Snapshot(enabled: false)), (_, _) => Task.CompletedTask);
        await model.LoadAsync();
        Assert.False(model.CodexEnabled);
        Assert.True(model.CanChange);
        Assert.False(model.IsVisible);
        Assert.Empty(model.SupportedProviders);
        Assert.False(model.CanRefresh("codex", null));
        await model.Refresh();
        Assert.False(model.IsRefreshing);
    }

    [Fact]
    public async Task AnExplicitRefreshWakesTheIdleObserverAndPollsUntilTheProbeEnds()
    {
        var gate = new Gate(Snapshot());
        var model = new SubscriptionUsageModel(gate.Read, activePollInterval: TimeSpan.FromMilliseconds(1));
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(false, cancel.Token);
        Assert.True(await gate.WaitForCalls(1));
        await Quiet();
        Assert.False(model.IsRefreshing);
        _ = model.Refresh();
        Assert.True(await gate.WaitForCalls(2));
        Assert.Equal((true, true), gate.Flags(1));
        gate.Resolve(1, Snapshot(refreshing: true));
        Assert.True(await gate.WaitForCalls(3));
        Assert.Equal((false, false), gate.Flags(2));
        gate.Resolve(2, Snapshot());
        await Settle(() => !model.IsRefreshing);
        Assert.False(model.IsRefreshing);
        cancel.Cancel();
        gate.ResolveRest(3, Snapshot());
        await observing;
    }

    [Fact]
    public async Task LeavingOverviewDiscardsAReadThatIgnoresCancellation()
    {
        var gate = new Gate();
        var model = new SubscriptionUsageModel(gate.Read);
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(false, cancel.Token);
        Assert.True(await gate.WaitForCalls(1));
        cancel.Cancel();
        gate.Resolve(0, Snapshot());
        await observing;
        Assert.Null(model.Shown);
        Assert.False(model.IsRefreshing);
    }

    [Fact]
    public async Task LeavingOverviewDropsAQueuedInteractiveRefresh()
    {
        var gate = new Gate();
        var model = new SubscriptionUsageModel(gate.Read);
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(false, cancel.Token);
        Assert.True(await gate.WaitForCalls(1));
        await model.LoadAsync(refresh: true, interactive: true);
        cancel.Cancel();
        gate.Resolve(0, Snapshot());
        await observing;
        await Quiet();
        int count = gate.Count;
        gate.ResolveRest(1, Snapshot());
        Assert.Equal(1, count);
        Assert.Null(model.Shown);
    }

    [Fact]
    public async Task APermissionFinishingAfterLeavingPublishesNothingAndStartsNoProbe()
    {
        var gate = new Gate();
        var written = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var writing = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var model = new SubscriptionUsageModel(gate.Read, (_, _) =>
        {
            writing.TrySetResult();
            return written.Task;
        });
        using var cancel = new CancellationTokenSource();
        Task observing = model.ObserveAsync(false, cancel.Token);
        Assert.True(await gate.WaitForCalls(1));
        Task enabling = model.SetEnabledAsync("codex", true);
        await writing.Task.WaitAsync(TimeSpan.FromSeconds(5));
        cancel.Cancel();
        await Quiet();
        written.SetResult();
        await Quiet();
        int count = gate.Count;
        gate.ResolveRest(1, Snapshot());
        await enabling;
        gate.Resolve(0, Snapshot());
        await observing;
        Assert.Equal(1, count);
        Assert.Empty(model.ChangingProviders);
        Assert.Null(model.Shown);
    }

    [Fact]
    public async Task AFailedReadDropsEarlierFigures()
    {
        int reads = 0;
        var model = new SubscriptionUsageModel((_, _, _, _) =>
            Interlocked.Increment(ref reads) == 1 ? Task.FromResult(Snapshot()) : throw new SubscriptionUsageException("x"));
        await model.LoadAsync();
        Assert.NotNull(model.Shown);
        await model.LoadAsync();
        Assert.Null(model.Shown);
        Assert.Equal("analytics_subscriptions_unavailable", model.FailureKey);
    }

    [Fact]
    public async Task AnotherSchemaIsTheUnavailableFailure()
    {
        var model = new SubscriptionUsageModel((_, _, _, _) => Task.FromResult(Snapshot() with { SchemaVersion = "trace_commons.subscription_usage.v2" }));
        await model.LoadAsync();
        Assert.Null(model.Shown);
        Assert.Equal("analytics_subscriptions_unavailable", model.FailureKey);
        Assert.False(model.Unsupported);
    }

    [Fact]
    public async Task ARefusedWriteSaysSoAndReadsTheSavedState()
    {
        var model = new SubscriptionUsageModel((_, _, _, _) => Task.FromResult(Snapshot()), (_, _) => throw new SubscriptionUsageException("refused"));
        await model.SetEnabledAsync("codex", false);
        Assert.Equal("analytics_subscriptions_change_failed", model.FailureKey);
        Assert.True(model.CodexEnabled);
        Assert.Empty(model.ChangingProviders);
    }

    // ---- The login words (InsightsSubscriptionsCard.swift) -------------------

    [Fact]
    public void ClaudeCaptureContextIsNeitherAFailureNorStale()
    {
        var login = new SubscriptionUsageLogin("claude-default", "Claude", "observed", Reason: "captured_on_refresh", UpdatedAt: Now, Stale: true);
        InsightsSubscriptionWords.Login words = InsightsSubscriptionWords.Of(login, "claude", Copy);
        Assert.Null(words.State);
        Assert.Null(words.Freshness);
    }

    [Fact]
    public void ClaudeRecoveryNamesTheCause()
    {
        foreach ((string reason, string key) in new[]
        {
            ("auth_expired", "claude_expired"), ("credentials_expired", "claude_expired"),
            ("auth_missing", "claude_auth_missing"), ("scope_missing", "claude_scope"),
            ("keychain_needs_permission", "claude_keychain_denied"),
            ("credential_read_timeout", "claude_keychain_pending"), ("auth_changed", "claude_auth_changed"),
            ("request_timeout", "request_timeout"), ("rate_limited", "rate_limited"),
        })
        {
            var login = new SubscriptionUsageLogin("claude-default", "Claude", "unknown", Reason: reason);
            string? state = InsightsSubscriptionWords.Of(login, "claude", Copy).State;
            Assert.Equal(Copy("analytics_subscriptions_" + key), state);
            Assert.False(string.IsNullOrEmpty(state));
        }
    }

    [Fact]
    public void PermanentFailuresNameTheProvider()
    {
        foreach (string provider in new[] { "claude", "codex" })
        {
            string name = Copy("analytics_subscriptions_provider_" + provider)!;
            foreach (string reason in new[] { "invalid_response", "no_logins", "base_url_refused", "native_storage_unsupported" })
            {
                var login = new SubscriptionUsageLogin(provider + "-default", name, "unknown", Reason: reason);
                string? state = InsightsSubscriptionWords.Of(login, provider, Copy).State;
                Assert.Equal(InsightsAnalyticsWords.Fill(Copy("analytics_subscriptions_" + reason)!, ("harness", name)), state);
                Assert.DoesNotContain("{harness}", state);
            }
        }

        Assert.Equal(Copy("analytics_subscriptions_native_tool_running"),
            InsightsSubscriptionWords.Of(new SubscriptionUsageLogin("c", "Codex", "unknown", Reason: "native_tool_running"), "codex", Copy).State);
        Assert.Equal(Copy("analytics_subscriptions_not_checked"),
            InsightsSubscriptionWords.Of(new SubscriptionUsageLogin("c", "Codex", "unknown", Reason: "not_checked"), "codex", Copy).State);
        Assert.Equal(Copy("analytics_subscriptions_unavailable"),
            InsightsSubscriptionWords.Of(new SubscriptionUsageLogin("c", "Codex", "unknown"), "codex", Copy).State);
        Assert.Null(InsightsSubscriptionWords.Of(new SubscriptionUsageLogin("c", "Codex", "observed"), "codex", Copy).State);
    }

    [Fact]
    public void AFailedRefreshKeepsTheLastReadingAndItsCause()
    {
        foreach (string reason in new[] { "native_tool_running", "invalid_response" })
        {
            var login = new SubscriptionUsageLogin("codex", "Codex", "observed", Reason: reason, Stale: true);
            InsightsSubscriptionWords.Login words = InsightsSubscriptionWords.Of(login, "codex", Copy);
            Assert.Equal(Copy("analytics_subscriptions_cached"), words.Freshness);
            Assert.NotNull(words.State);
        }

        var stale = new SubscriptionUsageLogin("codex", "Codex", "observed", Stale: true);
        Assert.Equal(Copy("analytics_subscriptions_stale"), InsightsSubscriptionWords.Of(stale, "codex", Copy).Freshness);
        Assert.Null(InsightsSubscriptionWords.Of(stale with { Stale = false }, "codex", Copy).Freshness);
    }

    [Fact]
    public void TheDefaultLoginIsNamedSoAndOthersByTheirLabel()
    {
        Assert.Equal(Copy("analytics_subscriptions_default_login"),
            InsightsSubscriptionWords.Of(new SubscriptionUsageLogin("a", "Work", "unknown", DefaultLogin: true), "codex", Copy).Label);
        Assert.Equal("Work", InsightsSubscriptionWords.Of(new SubscriptionUsageLogin("a", "Work", "unknown"), "codex", Copy).Label);
    }

    // ---- The rows and pace (InsightsLimitsTableTests.swift) ------------------

    private static readonly DateTimeOffset RowsNow = DateTimeOffset.FromUnixTimeSeconds(1_800_000_000);

    private static readonly Dictionary<string, string> RowCopy = new()
    {
        ["analytics_unavailable"] = "—", ["analytics_subscriptions_provider_codex"] = "Codex",
        ["analytics_subscriptions_provider_claude"] = "Claude", ["analytics_limits_on_pace"] = "On pace",
        ["analytics_limits_runs_out"] = "Runs out in {time}",
        ["analytics_subscriptions_default_login"] = "Default login",
        ["analytics_limits_headline_on_pace"] = "On pace: lasts until reset",
        ["analytics_limits_headline_runs_out"] = "{window} limit runs out in {time}, before it resets",
        ["analytics_subscriptions_weekly"] = "Weekly", ["analytics_subscriptions_session"] = "Session",
    };

    private static string? Rc(string key) => RowCopy.TryGetValue(key, out string? word) ? word : null;

    private static SubscriptionUsageWindow Weekly(int used, double hoursLeft) =>
        new("weekly", UsedPermille: used, ResetsAt: RowsNow.AddHours(hoursLeft), DurationMins: 10_080);

    [Fact]
    public void UsingLessThanTheTimePassedIsOnPace()
    {
        InsightsLimitsRows.PaceReading? pace = InsightsLimitsRows.Pace(Weekly(250, 84), RowsNow);
        Assert.NotNull(pace);
        Assert.True(pace!.OnPace);
    }

    [Fact]
    public void UsingFasterThanTheTimePassedRunsOutBeforeTheReset()
    {
        InsightsLimitsRows.PaceReading? pace = InsightsLimitsRows.Pace(Weekly(500, 126), RowsNow);
        Assert.Equal(RowsNow.AddHours(42), pace!.RunsOut);
    }

    [Fact]
    public void NoPaceWithoutEnoughOfTheWindow()
    {
        Assert.Null(InsightsLimitsRows.Pace(Weekly(100, 167), RowsNow));
        Assert.Null(InsightsLimitsRows.Pace(new SubscriptionUsageWindow("weekly", UsedPermille: 500), RowsNow));
        Assert.Equal(RowsNow, InsightsLimitsRows.Pace(Weekly(1_000, 10), RowsNow)!.RunsOut);
        Assert.True(InsightsLimitsRows.Pace(Weekly(0, 84), RowsNow)!.OnPace);
    }

    [Fact]
    public void RowsFillOnlyMeasuredCells()
    {
        var codex = new SubscriptionUsageProvider("codex", true, Logins: new[]
        {
            new SubscriptionUsageLogin("a", "me@example.com", "observed", DefaultLogin: true, Windows: new[]
            {
                new SubscriptionUsageWindow("session", UsedPermille: 400, ResetsAt: RowsNow.AddHours(1), DurationMins: 300),
                Weekly(250, 84),
            }),
            new SubscriptionUsageLogin("b", "other", "unavailable", Reason: "auth_expired"),
        });
        var claude = new SubscriptionUsageProvider("claude", true, Logins: new[]
        {
            new SubscriptionUsageLogin("c", "c", "observed", Windows: new[] { Weekly(500, 126) }),
        });
        IReadOnlyList<InsightsLimitsRows.Row> all = InsightsLimitsRows.Rows(new[] { codex, claude }, RowsNow, Rc);
        Assert.Equal(new[] { "Codex · Default login", "Codex · other", "Claude" }, all.Select(r => r.Account));
        InsightsLimitsRows.Row unmeasured = all[1];
        Assert.False(unmeasured.Measured);
        Assert.Equal(new[] { "—", "—", "—" }, new[] { unmeasured.Session, unmeasured.Weekly, unmeasured.Pace });
        Assert.Null(unmeasured.SessionReset);
        Assert.Null(unmeasured.WeeklyReset);
        Assert.False(unmeasured.PaceWarns);
        Assert.Null(unmeasured.Headline);
        Assert.Equal("b", unmeasured.LoginId);
        Assert.True(all[0].Measured);
        Assert.Equal("60%", all[0].Session);
        Assert.Equal("75%", all[0].Weekly);
        Assert.Equal("On pace", all[0].Pace);
        Assert.Equal("On pace: lasts until reset", all[0].Headline);
        Assert.False(all[0].PaceWarns);
        Assert.Null(all[2].Session);
        Assert.Null(all[2].SessionReset);
        Assert.Equal("50%", all[2].Weekly);
        Assert.NotNull(all[2].WeeklyReset);
        Assert.StartsWith("Runs out in ", all[2].Pace, StringComparison.Ordinal);
        Assert.Equal("Weekly limit runs out in 1d 18h, before it resets", all[2].Headline);
        Assert.True(all[2].PaceWarns);
    }

    [Fact]
    public void ManyLoginsNameTheDefaultAndKeepTheirRefreshTargets()
    {
        foreach (string provider in new[] { "claude", "codex" })
        {
            string name = RowCopy["analytics_subscriptions_provider_" + provider];
            IReadOnlyList<InsightsLimitsRows.Row> rows = InsightsLimitsRows.Rows(new[]
            {
                new SubscriptionUsageProvider(provider, true, Logins: new[]
                {
                    new SubscriptionUsageLogin("default", name, "unknown", DefaultLogin: true),
                    new SubscriptionUsageLogin("personal-id", "Personal", "unknown"),
                }),
            }, RowsNow, Rc);
            Assert.Equal(new[] { $"{name} · Default login", $"{name} · Personal" }, rows.Select(r => r.Account));
            Assert.Equal(new[] { "default", "personal-id" }, rows.Select(r => r.LoginId));
            Assert.Equal(new[] { "Default login", "Personal" }, rows.Select(r => r.LoginLabel));
            Assert.Equal(new[] { provider, provider }, rows.Select(r => r.Provider));
            Assert.Equal(2, rows.Select(r => r.Id).Distinct().Count());
        }
    }

    [Fact]
    public void OneLoginIsTitledByItsProviderAndKeepsItsRefreshTarget()
    {
        foreach (string provider in new[] { "claude", "codex" })
        {
            string name = RowCopy["analytics_subscriptions_provider_" + provider];
            foreach (bool isDefault in new[] { true, false })
            {
                var login = new SubscriptionUsageLogin(isDefault ? "default" : "personal-id", "Personal", "unknown", DefaultLogin: isDefault);
                IReadOnlyList<InsightsLimitsRows.Row> rows = InsightsLimitsRows.Rows(new[] { new SubscriptionUsageProvider(provider, true, Logins: new[] { login }) }, RowsNow, Rc);
                Assert.Equal(new[] { name }, rows.Select(r => r.Account));
                Assert.Equal(new[] { login.Id }, rows.Select(r => r.LoginId));
                Assert.Equal(new[] { isDefault ? "Default login" : "Personal" }, rows.Select(r => r.LoginLabel));
            }
        }
    }

    [Fact]
    public void EachWindowHasItsOwnResetAndBankedResetsShowOnlyWhenKnown()
    {
        var both = new SubscriptionUsageProvider("claude", true, Logins: new[]
        {
            new SubscriptionUsageLogin("c", "c", "observed", Windows: new[]
            {
                new SubscriptionUsageWindow("session", UsedPermille: 400, ResetsAt: RowsNow.AddHours(2), DurationMins: 300),
                Weekly(250, 84),
            }),
        });
        InsightsLimitsRows.Row row = InsightsLimitsRows.Rows(new[] { both }, RowsNow, Rc)[0];
        Assert.Equal("2h", row.SessionReset);
        Assert.Equal("3d 12h", row.WeeklyReset);
        Assert.Null(row.Banked);
        var codex = new SubscriptionUsageProvider("codex", true, Logins: new[]
        {
            new SubscriptionUsageLogin("x", "x", "observed", Windows: new[] { Weekly(100, 100) }, BankedResets: 2),
        });
        InsightsLimitsRows.Row codexRow = InsightsLimitsRows.Rows(new[] { codex }, RowsNow, Rc)[0];
        Assert.Equal("2", codexRow.Banked);
        Assert.Null(codexRow.Session);
    }

    /// <summary>An expired window is not a measurement: its rows are the dash, never a full allowance.</summary>
    [Fact]
    public void AnExpiredWindowReadsAsUnknown()
    {
        var provider = new SubscriptionUsageProvider("codex", true, Logins: new[]
        {
            new SubscriptionUsageLogin("x", "x", "observed", Stale: true, Windows: new[]
            {
                new SubscriptionUsageWindow("weekly", UsedPermille: 900, ResetsAt: RowsNow, DurationMins: 10_080),
            }),
        });
        InsightsLimitsRows.Row row = InsightsLimitsRows.Rows(new[] { provider }, RowsNow, Rc)[0];
        Assert.Equal("—", row.Weekly);
        Assert.Equal("—", row.WeeklyReset);
        Assert.Equal("—", row.Pace);
    }

    [Fact]
    public void LanesFollowTheWindowsLengthBeforeItsName()
    {
        Assert.True(InsightsLimitsRows.IsWeekly(new SubscriptionUsageWindow("secondary", DurationMins: 10_080)));
        Assert.True(InsightsLimitsRows.IsWeekly(new SubscriptionUsageWindow("weekly")));
        Assert.False(InsightsLimitsRows.IsWeekly(new SubscriptionUsageWindow("weekly", DurationMins: 300)));
        Assert.False(InsightsLimitsRows.IsWeekly(new SubscriptionUsageWindow("model", DurationMins: 10_080)));
        Assert.True(InsightsLimitsRows.IsSession(new SubscriptionUsageWindow("session")));
        Assert.True(InsightsLimitsRows.IsSession(new SubscriptionUsageWindow("primary", DurationMins: 300)));
        Assert.False(InsightsLimitsRows.IsSession(new SubscriptionUsageWindow("primary")));
        Assert.False(InsightsLimitsRows.IsSession(new SubscriptionUsageWindow("model", DurationMins: 300)));
    }

    [Fact]
    public void LeftRoundsHalfAwayFromZero()
    {
        Assert.Equal("58%", InsightsLimitsRows.Left(new SubscriptionUsageWindow("weekly", UsedPermille: 420)));
        Assert.Equal("1%", InsightsLimitsRows.Left(new SubscriptionUsageWindow("weekly", UsedPermille: 995)));
        Assert.Equal("3%", InsightsLimitsRows.Left(new SubscriptionUsageWindow("weekly", UsedPermille: 975)));
        Assert.Equal("0%", InsightsLimitsRows.Left(new SubscriptionUsageWindow("weekly", UsedPermille: 1_000)));
        Assert.Equal("", InsightsLimitsRows.Left(new SubscriptionUsageWindow("weekly")));
    }

    [Fact]
    public void DurationIsTheTwoLargestUnitsAndNeverUnderAMinute()
    {
        Assert.Equal("4d 21h", InsightsLimitsRows.Duration(RowsNow, RowsNow.AddHours(4 * 24 + 21).AddMinutes(30)));
        Assert.Equal("3h 5m", InsightsLimitsRows.Duration(RowsNow, RowsNow.AddHours(3).AddMinutes(5)));
        Assert.Equal("12m", InsightsLimitsRows.Duration(RowsNow, RowsNow.AddMinutes(12)));
        Assert.Equal("1m", InsightsLimitsRows.Duration(RowsNow, RowsNow.AddSeconds(5)));
        Assert.Equal("1m", InsightsLimitsRows.Duration(RowsNow, RowsNow.AddHours(-1)));
        Assert.Equal("2d", InsightsLimitsRows.Duration(RowsNow, RowsNow.AddDays(2)));
    }

    // ---- The bars (InsightsCreditTests.swift:62-75) -------------------------

    [Fact]
    public void ClaudeAndCodexBarsAreTokensPerDayWithUnknownDaysAsDashes()
    {
        var days = new[]
        {
            new InsightsUsageDay("2026-10-27", 1_500, null),
            new InsightsUsageDay("2026-10-28", 0, 2_000_000),
        };
        IReadOnlyList<InsightsUsageWords.Day> claude = InsightsUsageWords.TokenBars(days, "claude_code", Rc);
        IReadOnlyList<InsightsUsageWords.Day> codex = InsightsUsageWords.TokenBars(days, "codex", Rc);
        Assert.Equal(new double?[] { 1_500, 0 }, claude.Select(d => d.Value));
        Assert.Equal(new double?[] { null, 2_000_000 }, codex.Select(d => d.Value));
        Assert.Equal("—", codex[0].Amount);
        Assert.Equal(InsightsAnalyticsWords.Figure(Rc, 2_000_000), codex[1].Amount);
        Assert.Equal(new[] { "2026-10-27", "2026-10-28" }, codex.Select(d => d.Id));
        Assert.Equal(new[] { "T", "W" }, codex.Select(d => d.Label));
        Assert.Empty(InsightsUsageWords.TokenBars(null, "codex", Rc));
    }
}
