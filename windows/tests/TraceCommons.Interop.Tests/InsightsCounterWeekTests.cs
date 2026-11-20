using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Insights feed T (W20-D1, taken (a)): the window asks the daemon for
/// <c>insights_week</c>, as the Mac does, and draws it only when it is on,
/// readable and carries the core's Overview; anything else is the saved feed
/// (feed S, the local reads). The two are never mixed. Ported from the Mac's
/// <c>InsightsWeekFeedTests</c> and <c>testTheWeekPickerAsksTheDaemonForAnISOWeek</c>
/// (macos/Tests/TraceCommonsAppTests/InsightsModelTests.swift:330-445, :888-892).
/// </summary>
public sealed class InsightsCounterWeekTests
{
    private const string Counted = """
        {"enabled": true, "feed": "counter_pass", "readable": true,
         "updated_at": "2026-10-08T09:59:12+00:00", "sessions_stored": 3,
         "iso_week": "2026-W41", "week_start": "2026-10-05",
         "comparable": false, "unavailable": "below_coverage_floor",
         "overview": {"feed": "counter_pass"},
         "rollup": {"feed": "counter_pass", "week_start": "2026-10-05",
           "coverage": {"known": 1, "partial": 1, "unknown": 1, "reasons": {}},
           "undated_sessions": 0, "sources": [], "by_model": [],
           "sessions": [{"source": "claude_code", "tokens": 37235, "state": "partial",
                         "reasons": ["some_turns_unknown"]}]}}
        """;

    private static InsightsCounterWeek Week(string json) =>
        InsightsCounterWeek.Parse(JsonDocument.Parse(json).RootElement)!;

    private static readonly Lazy<Dictionary<string, string>> CoreCopy = new(() =>
    {
        var copy = new LocalInsights(Path.Combine(Path.GetTempPath(), "insights-copy-" + Guid.NewGuid().ToString("N")))
            .CallAsync(new { type = "copy" }, CancellationToken.None).GetAwaiter().GetResult();
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in copy.GetProperty("copy").EnumerateObject()) words[pair.Name] = pair.Value.GetString()!;
        return words;
    });

    private static string W(string key) => CoreCopy.Value[key];

    private static string N(ulong value) => value.ToString("N0", CultureInfo.CurrentCulture);

    // MARK: the reply

    [Fact]
    public void AnEnabledReadableWeekWithTheCoresOverviewShowsFeedT()
    {
        InsightsCounterWeek week = Week(Counted);
        Assert.True(week.ShowsCounterPass);
        Assert.Equal("2026-W41", week.IsoWeek);
        InsightsCounterSession row = Assert.Single(week.Sessions!);
        Assert.Equal(37235UL, row.Tokens);
        Assert.Null(row.ByDay);
    }

    [Theory]
    [InlineData("{\"enabled\": false, \"feed\": \"counter_pass\"}")]
    [InlineData("{\"enabled\": true, \"feed\": \"counter_pass\", \"readable\": false, \"reason\": \"store_unreadable\"}")]
    [InlineData("{\"enabled\": true, \"feed\": \"counter_pass\", \"readable\": false, \"overview\": {}, \"rollup\": {\"sessions\": []}}")]
    [InlineData("{\"enabled\": false, \"feed\": \"counter_pass\", \"readable\": true, \"overview\": {}, \"rollup\": {\"sessions\": []}}")]
    [InlineData("{\"enabled\": true, \"feed\": \"saved\", \"readable\": true, \"overview\": {}, \"rollup\": {\"sessions\": []}}")]
    [InlineData("{\"enabled\": true, \"feed\": \"counter_pass\", \"readable\": true, \"rollup\": {\"sessions\": []}}")]
    [InlineData("{\"enabled\": true, \"feed\": \"counter_pass\", \"readable\": true, \"overview\": {}}")]
    public void AnythingElseIsNotDrawnAsFeedT(string json)
    {
        Assert.False(Week(json).ShowsCounterPass);
    }

    [Fact]
    public void AnUnknownFigureStaysUnknownAndANegativeDayIsZero()
    {
        InsightsCounterWeek week = Week("""
            {"enabled": true, "feed": "counter_pass", "readable": true, "overview": {},
             "rollup": {"sessions": [{"state": "unknown", "reasons": ["no_usage_counters"], "tokens": null, "by_day": null},
                                     {"state": "known", "reasons": [], "tokens": 5, "by_day": [5, -1, 0, 0, 0, 0, 0]}]}}
            """);
        Assert.Null(week.Sessions![0].Tokens);
        Assert.Equal(new ulong[] { 5, 0, 0, 0, 0, 0, 0 }, week.Sessions[1].ByDay);
    }

    [Fact]
    public void AMalformedReplyIsNoWeek()
    {
        Assert.Null(InsightsCounterWeek.Parse(JsonDocument.Parse("{\"feed\": \"counter_pass\"}").RootElement));
        Assert.Null(InsightsCounterWeek.Parse(JsonDocument.Parse("[]").RootElement));
        Assert.Null(InsightsCounterWeek.Parse(JsonDocument.Parse(
            "{\"enabled\": true, \"feed\": \"counter_pass\", \"rollup\": {\"sessions\": [{\"tokens\": 1}]}}").RootElement));
    }

    [Fact]
    public void TheWeekPickerAsksTheDaemonForAnIsoWeek()
    {
        Assert.Equal("2026-W41", InsightsCounterWeek.IsoWeekOf("2026-10-05"));
        Assert.Equal("2026-W01", InsightsCounterWeek.IsoWeekOf("2025-12-29"));
        Assert.Null(InsightsCounterWeek.IsoWeekOf("last week"));
    }

    [Fact]
    public void TheRequestCarriesTheOffsetAndTheWeekOnlyWhenAsked()
    {
        using JsonDocument current = JsonDocument.Parse(InsightsCounterWeek.Parameters(null, -18000));
        Assert.Equal(new[] { "tz" }, current.RootElement.EnumerateObject().Select(p => p.Name));
        Assert.Equal(-18000, current.RootElement.GetProperty("tz").GetInt32());
        using JsonDocument asked = JsonDocument.Parse(InsightsCounterWeek.Parameters("2026-W40", 3600));
        Assert.Equal("2026-W40", asked.RootElement.GetProperty("iso_week").GetString());
        Assert.Equal(3600, asked.RootElement.GetProperty("tz").GetInt32());
    }

    [Fact]
    public void ACountedSessionIsTitledByItsFolderAndReference()
    {
        Func<string, string?> copy = key => key == "analytics_folder_unknown" ? "Unknown folder" : null;
        var known = new InsightsCounterSession("claude_code", "infra", "t1", 1, "known", Array.Empty<string>(), null, null, null);
        Assert.Equal("infra · t1", InsightsCounterWeek.Title(known, copy));
        Assert.Equal("Unknown folder", InsightsCounterWeek.Title(known with { Folder = "", SessionRef = null }, copy));
    }

    // MARK: the model

    /// <summary>A local store that answers the saved feed, so a mix would show.</summary>
    private sealed class Local : ILocalInsights
    {
        public List<string> Types { get; } = new();

        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            JsonElement request = JsonSerializer.SerializeToElement(operation);
            string type = request.GetProperty("type").GetString()!;
            Types.Add(type);
            string name = type switch
            {
                "week_overview" => "week_overview.json",
                "patterns" => "patterns.json",
                "card_inputs" => "card_inputs_tokens.json",
                _ => "",
            };
            if (type == "copy")
            {
                return Task.FromResult(JsonSerializer.SerializeToElement(new { type = "copy", copy = CoreCopy.Value }));
            }

            if (type == "list")
            {
                return Task.FromResult(JsonDocument.Parse("{\"type\":\"list\",\"insights\":[]}").RootElement.Clone());
            }

            if (type is "summary" or "episode_list")
            {
                throw new InvalidOperationException("insights-operation-failed");
            }

            return Task.FromResult(JsonDocument.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "insights-analytics", name))).RootElement.Clone());
        }

        public int Count(string type) => Types.Count(t => t == type);
    }

    private sealed class Reader
    {
        private readonly Queue<Func<DaemonResponse>> _answers = new();

        public List<string?> Asked { get; } = new();

        public Reader Then(string result)
        {
            _answers.Enqueue(() => DaemonResponse.Parse("{\"id\":1,\"result\":" + result + "}"));
            return this;
        }

        public Reader ThenError(string code, string message)
        {
            _answers.Enqueue(() => DaemonResponse.Parse("{\"id\":1,\"error\":{\"code\":\"" + code + "\",\"message\":\"" + message + "\"}}"));
            return this;
        }

        public Task<DaemonResponse> Read(string? isoWeek)
        {
            Asked.Add(isoWeek);
            return Task.FromResult(_answers.Count > 1 ? _answers.Dequeue()() : _answers.Peek()());
        }
    }

    private static string SampleWeek => InsightsSample.Week.GetRawText();

    private static async Task<(InsightsViewModel Model, Local Local)> Open(Reader? reader)
    {
        var local = new Local();
        var model = new InsightsViewModel(local, weekReader: reader is null ? null : reader.Read);
        await model.LoadAsync();
        return (model, local);
    }

    [Fact]
    public async Task OpeningInsightsAsksTheDaemonForTheCurrentWeek()
    {
        var reader = new Reader().Then(SampleWeek);
        var (model, _) = await Open(reader);
        using var _model = model;
        Assert.Equal(new string?[] { null }, reader.Asked);
        Assert.True(model.CounterFeed);
        Assert.Equal("", model.CounterNotice);
    }

    [Fact]
    public async Task FeedTReplacesTheSavedWeekAndIsNeverMixedWithIt()
    {
        var (model, local) = await Open(new Reader().Then(SampleWeek));
        using var _model = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        InsightsOverviewViewModel overview = model.Overview;
        Assert.Equal(0, local.Count("week_overview"));
        Assert.Equal("37", overview.SessionsFigure);
        Assert.Equal(new[] { N(45_000_000), N(6_200_000) }, overview.TokenLines.Select(l => l.Figure));
        Assert.Equal(W("analytics_feed_counter_pass"), overview.Notes.Last());
        // "vs last week" and "Your best week" are the core's figures under feed T.
        Assert.Equal(InsightsAnalyticsWords.Fill(W("analytics_change_down"), ("p", "12")), overview.TokenLines[0].Change);
        Assert.Equal(InsightsAnalyticsWords.Fill(W("analytics_change_up"), ("p", "4")), overview.TokenLines[1].Change);
        Assert.Equal(InsightsAnalyticsWords.Fill(W("analytics_best_week"), ("q", "84")), overview.CacheLines[0].Best);
        Assert.Equal(W("analytics_unavailable"), overview.CacheLines[1].Best);
        Assert.Equal(7, overview.Days.Count);
        Assert.Equal(5, overview.Models.Count);
        Assert.Equal("2026-10-05", overview.Weeks[overview.SelectedWeekIndex].Value);
        Assert.Equal(5, overview.Weeks.Count);

        await model.SelectTabAsync(InsightsViewModel.PatternsTab);
        Assert.Equal(0, local.Count("patterns"));
        Assert.Equal("2026-10-05", model.Patterns.Weeks[model.Patterns.SelectedWeekIndex].Value);
        Assert.Contains(W("analytics_feed_counter_pass"), model.Patterns.Notes);
    }

    [Fact]
    public async Task UnderFeedTTheSessionsBreakdownIsTheDaemonsCountedSessions()
    {
        var (model, local) = await Open(new Reader().Then(SampleWeek));
        using var _model = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        await model.Overview.ToggleInputsAsync("sessions");
        Assert.Equal(0, local.Count("card_inputs"));
        Assert.Equal("sessions", model.Overview.OpenCard);
        Assert.Equal(37, model.Overview.Inputs.Count);
        AnalyticsTableRow first = model.Overview.Inputs[0];
        Assert.StartsWith("infra · t1", first.Session);
        Assert.Equal(N(960_000), first.Tokens);
        Assert.Equal(W("analytics_state_known"), first.Coverage);
        AnalyticsTableRow unknown = model.Overview.Inputs[11];
        Assert.Equal(W("analytics_unavailable"), unknown.Tokens);
        Assert.Equal(W("analytics_state_unknown"), unknown.Coverage);
        Assert.Equal(W("analytics_reason_no_usage_counters"), unknown.Reason);

        // The other figures read no saved breakdown under the daemon's week.
        await model.Overview.ToggleInputsAsync("tokens");
        await model.Overview.ToggleInputsAsync("cache_share");
        Assert.Equal(0, local.Count("card_inputs"));
        Assert.Equal("sessions", model.Overview.OpenCard);
    }

    [Fact]
    public async Task SwitchedOffOrAnOlderDaemonShowsTheSavedWeekWithoutANotice()
    {
        var readers = new Reader?[]
        {
            null,
            new Reader().Then("{\"enabled\": false, \"feed\": \"counter_pass\"}"),
            new Reader().ThenError("unknown_method", "unknown-method"),
        };
        foreach (Reader? reader in readers)
        {
            var (model, local) = await Open(reader);
            using var _model = model;
            Assert.False(model.CounterFeed);
            Assert.Equal("", model.CounterNotice);
            await model.SelectTabAsync(InsightsViewModel.OverviewTab);
            Assert.Equal(1, local.Count("week_overview"));
            Assert.Equal("2", model.Overview.SessionsFigure);
            Assert.DoesNotContain(W("insights_feed_counter_pass_unavailable"), model.Overview.Notes);
        }
    }

    [Fact]
    public async Task AFailedOrUnreadableReadShowsTheSavedWeekAndSaysCountingIsUnavailable()
    {
        var readers = new[]
        {
            new Reader().Then("{\"enabled\": true, \"feed\": \"counter_pass\", \"readable\": false, \"reason\": \"store_unreadable\"}"),
            new Reader().ThenError("unavailable", "daemon-not-running"),
            new Reader().Then("{\"enabled\": true}"),
            new Reader().Then(Counted.Replace("\"overview\": {\"feed\": \"counter_pass\"},", "", StringComparison.Ordinal)),
        };
        foreach (Reader reader in readers)
        {
            var (model, _) = await Open(reader);
            using var _model = model;
            Assert.False(model.CounterFeed);
            Assert.Equal(W("insights_feed_counter_pass_unavailable"), model.CounterNotice);
            await model.SelectTabAsync(InsightsViewModel.OverviewTab);
            Assert.Equal("2", model.Overview.SessionsFigure);
            Assert.Contains(W("insights_feed_counter_pass_unavailable"), model.Overview.Notes);
        }
    }

    [Fact]
    public async Task FeedTIsDroppedWholeWhenALaterReadFails()
    {
        var reader = new Reader().Then(SampleWeek).ThenError("unavailable", "daemon-not-running");
        var (model, local) = await Open(reader);
        using var _model = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        Assert.True(model.CounterFeed);
        Assert.Equal("37", model.Overview.SessionsFigure);

        // Under feed T the week picker asks the daemon for an ISO week.
        int index = model.Overview.Weeks.ToList().FindIndex(w => w.Value == "2026-09-28");
        await model.Overview.SelectWeekAsync(index);
        Assert.Equal(new string?[] { null, "2026-W40" }, reader.Asked);
        Assert.False(model.CounterFeed);
        Assert.Null(model.CounterWeek);

        // Never a T figure under the S feed line: the saved week is read again.
        Assert.Equal("2", model.Overview.SessionsFigure);
        Assert.True(local.Count("week_overview") >= 1);
        Assert.DoesNotContain(W("analytics_feed_counter_pass"), model.Overview.Notes);
    }

    [Fact]
    public async Task UnderTheSavedFeedTheWeekPickerReadsTheStore()
    {
        var reader = new Reader().Then("{\"enabled\": false, \"feed\": \"counter_pass\"}");
        var (model, local) = await Open(reader);
        using var _model = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        await model.Overview.SelectWeekAsync(0);
        Assert.Single(reader.Asked);
        Assert.Equal(2, local.Count("week_overview"));
    }

    [Fact]
    public async Task ASavedWeekReadThatLandsLateNeverReplacesFeedT()
    {
        var gate = new TaskCompletionSource<JsonElement>();
        var slow = new SlowLocal(gate.Task);
        var reader = new Reader().Then(SampleWeek);
        var model = new InsightsViewModel(slow, weekReader: reader.Read);
        using var _model = model;
        Task overview = model.Overview.LoadAsync();
        await model.LoadWeekAsync();
        Assert.Equal("37", model.Overview.SessionsFigure);
        gate.SetResult(JsonDocument.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "insights-analytics", "week_overview.json"))).RootElement.Clone());
        await overview;
        Assert.Equal("37", model.Overview.SessionsFigure);
    }

    private sealed class SlowLocal : ILocalInsights
    {
        private readonly Task<JsonElement> _week;

        public SlowLocal(Task<JsonElement> week) => _week = week;

        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            string type = JsonSerializer.SerializeToElement(operation).GetProperty("type").GetString()!;
            return type == "copy"
                ? Task.FromResult(JsonSerializer.SerializeToElement(new { type = "copy", copy = CoreCopy.Value }))
                : _week;
        }
    }

    // MARK: the sample week

    [Fact]
    public void TheSampleWeekIsTheMacsWeek()
    {
        InsightsCounterWeek week = InsightsCounterWeek.Parse(InsightsSample.Week)!;
        Assert.True(week.ShowsCounterPass);
        Assert.Equal("2026-W41", week.IsoWeek);
        Assert.Equal("2026-W41", InsightsCounterWeek.IsoWeekOf(week.WeekStart!));
        Assert.Equal(37, week.Sessions!.Count);
        Assert.Equal(new[] { 34, 2, 1 }, new[] { "known", "partial", "unknown" }.Select(s => week.Sessions.Count(r => r.State == s)));
        Assert.Equal(Enumerable.Range(1, 37).Select(i => "t" + i), week.Sessions.Select(r => r.SessionRef));

        // The Claude Code sessions' days sum exactly to the week's by_day
        // (InsightsSampleWeek.swift:132-135).
        JsonElement byDay = week.Overview!.Value.GetProperty("by_day");
        ulong[] totals = byDay.EnumerateArray().Select(d =>
            d.GetProperty("uncached").GetUInt64() + d.GetProperty("cache_read").GetUInt64()
            + d.GetProperty("cache_write").GetUInt64() + d.GetProperty("output").GetUInt64()).ToArray();
        ulong[] sums = Enumerable.Range(0, 7).Select(day => week.Sessions
            .Where(r => r.Source == "claude_code" && r.ByDay is not null)
            .Aggregate(0UL, (sum, r) => sum + r.ByDay![day])).ToArray();
        Assert.Equal(totals, sums);
        Assert.Equal(45_000_000UL, week.Overview.Value.GetProperty("sources")[0].GetProperty("tokens").GetUInt64());
        Assert.Equal(872, week.Overview.Value.GetProperty("sources")[0].GetProperty("cache_share").GetProperty("permille").GetInt32());
    }

    [Fact]
    public async Task ASampleWindowDrawsTheSampleWeekAndReadsNothingFromTheStore()
    {
        var model = new InsightsViewModel(new SampleInsights(), weekReader: InsightsViewModel.SampleWeekReader);
        using var _model = model;
        await model.LoadAsync();
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        Assert.True(model.CounterFeed);
        Assert.False(model.Overview.Failed);
        Assert.Equal("37", model.Overview.SessionsFigure);
        Assert.Equal(5, model.Overview.Weeks.Count);
        Assert.NotEqual("", model.Overview.CoverageLine);
        await model.SelectTabAsync(InsightsViewModel.PatternsTab);
        Assert.False(model.Patterns.Failed);
        Assert.Equal(5, model.Patterns.Weeks.Count);
    }
}
