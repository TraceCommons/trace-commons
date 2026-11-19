using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Private AI tab's ledger sections driven by the recorded samples
/// (spec W19, InferenceViews.swift:91-117 and :286-325), as the box draws them.
/// </summary>
public sealed class InferenceLedgerModelTests
{
    private static readonly string Root = Path.Combine(AppContext.BaseDirectory, "RecordedSamples");

    private static InferenceLedgerModel Model(string set, List<string>? sent = null)
    {
        var daemon = new SampleDaemon(set, Root);
        RoutingCopy? routing = RoutingSurface.Copy();
        var insights = LocalInsights.CoreCopy();
        return new InferenceLedgerModel(
            (method, parameters) =>
            {
                sent?.Add(method + " " + parameters);
                return Task.FromResult(DaemonResponse.Parse(daemon.Call(method, parameters)));
            },
            MonitorShellCopy.Top,
            key => insights.TryGetValue(key, out string? w) ? w : string.Empty,
            id => id switch
            {
                "claude-code" => routing?.ToolClaude,
                "codex" => routing?.ToolCodex,
                _ => null,
            },
            TimeZoneInfo.Utc,
            CultureInfo.GetCultureInfo("en-US"));
    }

    [Fact]
    public async Task NormalDayDrawsTheWindowThePricedTotalAndEveryCall()
    {
        var sent = new List<string>();
        InferenceLedgerModel model = Model("normalDay", sent);
        Assert.True(model.Loading);
        await model.LoadAsync();

        // The three reads the Mac sends, the calls one page of 50 (InferenceStore.swift:229).
        Assert.Equal(new[] { "inference_calls {\"limit\":50}", "inference_summary {}", "tool_destinations {}" }, sent.OrderBy(s => s, StringComparer.Ordinal));
        Assert.False(model.Loading);
        Assert.False(model.HasFailure);
        Assert.True(model.ShowsHeader);
        Assert.Equal(MonitorShellCopy.Top("window_last_hours").Replace("{hours}", "24"), model.WindowLine);
        // Without a network summary only priced is drawn, and one unknown
        // price makes it the dash (probe: totals nil 1 —).
        Assert.Equal(("—", "—", "—"), (model.CallsFigure, model.VerifiedFigure, model.PricedFigure));
        Assert.Equal(new[] { "Calls", "Verified", "Priced" }, new[] { model.CallsWord, model.VerifiedWord, model.PricedWord });

        Assert.True(model.ShowsCalls);
        Assert.Equal(new long[] { 414, 413, 412, 411, 410 }, model.Calls.Select(c => c.Id));
        LedgerCallRow first = model.Calls[0];
        Assert.Equal("Claude Code", first.Tool);
        Assert.Equal("zai-org/GLM-4.6", first.Model);
        Assert.Equal("$0.0084", first.Priced);
        Assert.Equal(new LedgerTag("Verified", LedgerTone.On), first.Proof);
        Assert.StartsWith("9:04", first.Time);
        LedgerCallRow outside = model.Calls[2];
        Assert.Equal("—", outside.Tool);
        Assert.Equal("—", outside.Priced);
        Assert.Equal(new LedgerTag("External", LedgerTone.Outside), outside.Proof);
        Assert.Equal(new LedgerTag("Gateway only", LedgerTone.Ask), model.Calls[1].Proof);
        Assert.Equal(new LedgerTag("Pending", LedgerTone.Ask), model.Calls[3].Proof);
        Assert.Equal(new LedgerTag("Check failed", LedgerTone.Failed), model.Calls[4].Proof);
        Assert.Contains("Unknown", outside.AccessibleName);
    }

    // An unreadable ledger draws no header, no table and no zero
    // (InferenceViews.swift:111-114).
    [Fact]
    public async Task AnUnreadableLedgerDrawsNothing()
    {
        InferenceLedgerModel model = Model("empty");
        await model.LoadAsync();
        Assert.False(model.ShowsHeader);
        Assert.False(model.ShowsCalls);
        Assert.False(model.HasFailure);
        Assert.False(model.Loading);
        Assert.Equal(string.Empty, model.WindowLine);
    }

    // The core down is said in the core's words, never the fixed label.
    [Fact]
    public async Task TheCoreDownIsTheCoresLine()
    {
        InferenceLedgerModel model = Model(SampleDaemon.CoreDown);
        await model.LoadAsync();
        Assert.True(model.HasFailure);
        Assert.Equal(MonitorShellCopy.Top("core_unreachable"), model.FailureLine);
        Assert.NotEqual(string.Empty, model.FailureLine);
        Assert.DoesNotContain("daemon", model.FailureLine, StringComparison.OrdinalIgnoreCase);
        Assert.False(model.Loading);
        Assert.False(model.ShowsHeader);
    }

    // A load answered after a newer one changes nothing.
    [Fact]
    public async Task AnOlderLoadIsDropped()
    {
        var daemon = new SampleDaemon("normalDay", Root);
        var gate = new TaskCompletionSource();
        int loads = 0;
        var model = new InferenceLedgerModel(
            async (method, parameters) =>
            {
                int load = loads;
                if (load == 1) await gate.Task;
                return DaemonResponse.Parse(method == "inference_calls" && load == 1
                    ? daemon.Call(method, parameters)
                    : new SampleDaemon("empty", Root).Call(method, parameters));
            },
            MonitorShellCopy.Top, _ => string.Empty, _ => null, TimeZoneInfo.Utc, CultureInfo.InvariantCulture);
        loads = 1;
        Task older = model.LoadAsync();
        loads = 2;
        await model.LoadAsync();
        gate.SetResult();
        await older;
        Assert.False(model.ShowsCalls);
    }

    // A daemon's refusal is a failed request, not the core down, whatever its
    // code (LiveDaemonClient.swift:613-616).
    [Fact]
    public async Task ARefusalIsAFailedRequestNotTheCoreDown()
    {
        var model = new InferenceLedgerModel(
            (method, parameters) => Task.FromResult(DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"ledger-unreadable\"}}")),
            MonitorShellCopy.Top, _ => string.Empty, _ => null, TimeZoneInfo.Utc, CultureInfo.InvariantCulture);
        await model.LoadAsync();
        Assert.Equal(MonitorShellCopy.Top("request_failed"), model.FailureLine);
        Assert.NotEqual(MonitorShellCopy.Top("core_unreachable"), model.FailureLine);
    }

    // A reply that does not decode is the Mac's `.undecodable`: a failed
    // request over the last page (LiveDaemonClient.swift:611-618,
    // InferenceStore.swift loadCalls keeps the last value), never a silent
    // success that keeps the spinner or the old page as current.
    [Fact]
    public async Task AReplyThatDoesNotDecodeIsAFailedRequestOverTheLastPage()
    {
        var daemon = new SampleDaemon("normalDay", Root);
        bool garbled = false;
        var model = new InferenceLedgerModel(
            (method, parameters) => Task.FromResult(DaemonResponse.Parse(garbled && method == "inference_calls"
                ? "{\"id\":1,\"result\":{\"calls\":\"not a page\"}}"
                : daemon.Call(method, parameters))),
            MonitorShellCopy.Top, _ => string.Empty, _ => null, TimeZoneInfo.Utc, CultureInfo.InvariantCulture);
        await model.LoadAsync();
        Assert.False(model.HasFailure);
        int calls = model.Calls.Count;
        Assert.True(calls > 0);

        garbled = true;
        await model.LoadAsync();
        Assert.Equal(MonitorShellCopy.Top("request_failed"), model.FailureLine);
        Assert.Equal(calls, model.Calls.Count);
        Assert.False(model.Loading);
    }

    // Before any page, an undecodable reply is the failure line, not a
    // spinner forever.
    [Fact]
    public async Task AFirstReplyThatDoesNotDecodeEndsTheSpinner()
    {
        var model = new InferenceLedgerModel(
            (method, parameters) => Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":[]}")),
            MonitorShellCopy.Top, _ => string.Empty, _ => null, TimeZoneInfo.Utc, CultureInfo.InvariantCulture);
        await model.LoadAsync();
        Assert.False(model.Loading);
        Assert.Equal(MonitorShellCopy.Top("request_failed"), model.FailureLine);
    }

    // Each of the Mac's six unreachable labels (LiveDaemonClient.swift:603-606)
    // is the core down, as are this shell's two for no daemon started.
    [Theory]
    [InlineData("daemon-stopped")]
    [InlineData("daemon-disconnected")]
    [InlineData("attached-transport-failed")]
    [InlineData("handle-freed")]
    [InlineData("null-handle")]
    [InlineData("invalid-handle-pointer")]
    [InlineData("daemon-not-started")]
    [InlineData("daemon-not-running")]
    public async Task EachUnreachableLabelIsTheCoreDown(string label)
    {
        var model = new InferenceLedgerModel(
            (method, parameters) => Task.FromResult(DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"" + label + "\"}}")),
            MonitorShellCopy.Top, _ => string.Empty, _ => null, TimeZoneInfo.Utc, CultureInfo.InvariantCulture);
        await model.LoadAsync();
        Assert.Equal(MonitorShellCopy.Top("core_unreachable"), model.FailureLine);
    }

    // The set is exactly those eight: an unfamiliar label is a failed request.
    [Fact]
    public void TheUnreachableSetIsTheMacsSixAndTheShellsTwo()
    {
        Assert.Equal(
            new[] { "attached-transport-failed", "daemon-disconnected", "daemon-not-running", "daemon-not-started",
                "daemon-stopped", "handle-freed", "invalid-handle-pointer", "null-handle" },
            InferenceLedgerModel.UnreachableMessages.OrderBy(s => s, StringComparer.Ordinal));
    }
}
