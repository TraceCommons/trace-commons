using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Benchmarks (spec W20) against the Mac's own expectations
/// (macos/Tests/TraceCommonsAppTests/InsightsWorkTests.swift:68-196) and the
/// Mac's sample (InsightsSample.swift:68-104), through the real core words.
/// </summary>
public sealed class InsightsBenchmarksTests
{
    private static readonly IReadOnlyDictionary<string, string> Copy = LocalInsights.CoreCopy();

    private static string W(string key) => Copy.TryGetValue(key, out string? word) ? word : string.Empty;

    private static BenchmarkPick Pick(string leader, string tool, long figure, string next, string nextTool, long nextFigure) =>
        new(new BenchmarkEntry(leader, tool, figure), new BenchmarkEntry(next, nextTool, nextFigure));

    // InsightsWorkTests.swift:68-88.
    [Fact]
    public void PicksAndTipsReadInTheCoresWords()
    {
        var tip = new BenchmarkTip("test_first", FasterPermille: 350, SessionsWithout: 12, Sessions: 34);
        var category = new BenchmarkCategory("bug_fix", 34, Array.Empty<BenchmarkModel>(),
            Pick("gpt-5.5", "codex", 930, "opus", "claude_code", 880),
            Pick("sonnet", "claude_code", 400_000, "gpt-5.5", "codex", 950_000),
            null, tip);
        IReadOnlyList<BenchmarkPickRow> picks = InsightsBenchmarkWords.Picks(category, W);
        Assert.Equal(new[] { "quality", "value" }, picks.Select(p => p.Id));
        Assert.Equal("93% kept", picks[0].Figure);
        Assert.Equal("Next: opus, 88% kept", picks[0].Next);
        Assert.Equal("$0.40 per trace", picks[1].Figure);
        Assert.Equal("Fix a bug", InsightsBenchmarkWords.Name("bug_fix", W));
        Assert.Equal("Fixes where a failing test ran first finished 35% faster. 12 of your 34 fixes did not start that way.",
            InsightsBenchmarkWords.Tip(tip, W));
        Assert.Null(InsightsBenchmarkWords.Tip(new BenchmarkTip("from_a_newer_daemon"), W));
    }

    // InsightsWorkTests.swift:93-101.
    [Fact]
    public void PopulationsAndTheIntroReadInTheCoresWords()
    {
        Assert.Equal(new[] { "own", "spawned" }, InsightsBenchmarkWords.Populations);
        Assert.Equal("Your traces", InsightsBenchmarkWords.Population("own", W));
        Assert.Equal("Sub-agents", InsightsBenchmarkWords.Population("spawned", W));
        Assert.Equal("Compared on 12 of your traces", InsightsBenchmarkWords.Intro(12, "own", W));
        Assert.Equal("Compared on 40 Claude sub-agent traces", InsightsBenchmarkWords.Intro(40, "spawned", W));
    }

    // InsightsWorkTests.swift:103-114.
    [Fact]
    public void TheAnswerDecodesItsPopulationAndAnOlderDaemonHasNone()
    {
        InsightsBenchmarks? answer = InsightsBenchmarks.Parse(JsonDocument.Parse(
            "{\"enabled\":true,\"readable\":true,\"weeks\":4,\"population\":\"spawned\",\"sessions\":3,\"min_sessions\":5,\"categories\":[],\"delegated\":0}").RootElement);
        Assert.Equal("spawned", answer!.Population);
        Assert.Equal(5, answer.MinSessions);
        InsightsBenchmarks? older = InsightsBenchmarks.Parse(JsonDocument.Parse("{\"enabled\":true,\"categories\":[]}").RootElement);
        Assert.Null(older!.Population);
        Assert.Null(InsightsBenchmarks.Parse(JsonDocument.Parse("{\"categories\":[]}").RootElement));
    }

    // InsightsWorkTests.swift:118-136: the model asks for the population and
    // period chosen; the sample has no reader and stays drawn.
    [Fact]
    public async Task TheModelAsksForTheChosenPopulationAndTheSampleAsksNothing()
    {
        var asked = new List<string>();
        var model = new InsightsBenchmarksModel((method, parameters) =>
        {
            asked.Add(method + " " + parameters);
            return Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":{\"enabled\":true,\"categories\":[]}}"));
        }, W);
        Assert.Equal("own", model.Population);
        await model.LoadAsync();
        await model.SelectAsync(13, "spawned");
        Assert.Equal(new[] { "insights_benchmarks {\"weeks\":4}", "insights_benchmarks {\"weeks\":13,\"population\":\"spawned\"}" }, asked);

        var sample = new InsightsBenchmarksModel(InsightsSample.Benchmarks, W);
        await sample.SelectAsync(13, "spawned");
        Assert.NotNull(sample.Answer);
        Assert.False(sample.Available);
    }

    // InsightsWorkTests.swift:140-162.
    [Fact]
    public void OneModelShowsItsOwnFiguresAndWorkMix()
    {
        var opus = new BenchmarkModel("claude-opus-5-5", "claude_code", 6, 1000, 4_785_380, 1_436);
        var docs = new BenchmarkModel("claude-opus-5-5", "claude_code", 2, null, null, null);
        var answer = new InsightsBenchmarks(true, true, 4, null, 8, 5, new[]
        {
            new BenchmarkCategory("docs", 2, new[] { docs }, null, null, null, null),
            new BenchmarkCategory("feature", 6, new[] { opus }, null, null, null, null),
        });
        IReadOnlyList<BenchmarkModelRow> rows = InsightsBenchmarkWords.Rows(answer.Categories![1], W);
        Assert.Equal(new[] { "100%" }, rows.Select(r => r.Kept));
        Assert.Equal(new[] { "$4.79" }, rows.Select(r => r.Cost));
        Assert.Equal(new[] { "6 traces" }, rows.Select(r => r.Traces));
        Assert.Equal(new[] { W("analytics_unavailable") }, InsightsBenchmarkWords.Rows(answer.Categories[0], W).Select(r => r.Kept));

        IReadOnlyList<BenchmarkMix> mix = InsightsBenchmarkWords.Mix(answer, W);
        Assert.Equal(new[] { "claude-opus-5-5" }, mix.Select(m => m.Model));
        Assert.Equal("8 traces", mix[0].Traces);
        Assert.Equal(new[] { "Build a feature", "Docs & writing" }, mix[0].Shares.Select(s => s.Task));
        Assert.Equal(new[] { "75% · 6 traces", "25% · 2 traces" }, mix[0].Shares.Select(s => s.Label));
    }

    // InsightsWorkTests.swift:166-178: the traces compared, not the window.
    [Fact]
    public void TheHeaderCountsTheTracesActuallyCompared()
    {
        var answer = new InsightsBenchmarks(true, true, 4, null, 1_382, 5, new[]
        {
            new BenchmarkCategory("bug_fix", 14, Array.Empty<BenchmarkModel>(), null, null, null, null),
            new BenchmarkCategory("feature", 8, Array.Empty<BenchmarkModel>(), null, null, null, null),
            new BenchmarkCategory("tests", 0, Array.Empty<BenchmarkModel>(), null, null, null, null),
        });
        Assert.Equal(22, InsightsBenchmarkWords.Compared(answer));
        Assert.Equal(0, InsightsBenchmarkWords.Compared(new InsightsBenchmarks(true, true, 4, null, 5, 5, null)));
    }

    // The Mac's sample (InsightsSample.swift:68-104) as the box draws it:
    // five cards in the tab's order, three picks each, the sample's tips.
    [Fact]
    public void TheMacsSampleDrawsFiveCardsWithItsFigures()
    {
        var model = new InsightsBenchmarksModel(InsightsSample.Benchmarks, W);
        IReadOnlyList<BenchmarkCard> cards = model.Cards;
        Assert.Equal(new[] { "bug_fix", "feature", "refactor", "tests", "docs" }, cards.Select(c => c.Key));
        Assert.Equal("34 traces", cards[0].Traces);
        Assert.Equal(new[] { "93% kept", "$0.41 per trace", "14m" }, cards[0].Picks.Select(p => p.Figure));
        Assert.Equal(new[] { "claude-opus-5-5", "claude-sonnet-5-5", "gpt-5-codex" }, cards[0].Picks.Select(p => p.Model));
        Assert.Equal("Next: claude-opus-5-5, $1.84 per trace", cards[0].Picks[1].Next);
        Assert.Equal("Next: claude-opus-5-5, 21m", cards[0].Picks[2].Next);
        Assert.Equal("Next: claude-opus-5-5, 35m", cards[1].Picks[2].Next);
        Assert.Equal("$0.06 per trace", cards[4].Picks[1].Figure);
        Assert.Equal("4m", cards[4].Picks[2].Figure);
        Assert.Equal("claude-sonnet-5-5 kept about as many edits as claude-opus-5-5 at 25% of the cost.", cards[1].Tip);
        Assert.Equal("gpt-5-codex had 2.3× fewer edits undone than claude-opus-5-5.", cards[2].Tip);
        Assert.Null(cards[4].Tip);
        Assert.All(cards, c => Assert.Null(c.CompareLater));
        // The sample has no per-model rows, so no mix card; and no intro.
        Assert.False(model.HasMixes);
        Assert.False(model.HasIntro);
        Assert.False(model.HasNote);
    }

    // Swift's DateComponentsFormatter (abbreviated, hours and minutes, two
    // units, zeros dropped) and the one-minute floor.
    [Theory]
    [InlineData(0L, "1m")]
    [InlineData(240L, "4m")]
    [InlineData(1_260L, "21m")]
    [InlineData(3_600L, "1h")]
    [InlineData(3_900L, "1h 5m")]
    public void DurationIsTheMacs(long seconds, string expected) =>
        Assert.Equal(expected, InsightsBenchmarkWords.Duration(seconds));

    // InsightsWorkWords.dollars: two decimals from a cent up, the money form under it.
    [Theory]
    [InlineData(410_000L, "$0.41")]
    [InlineData(4_785_380L, "$4.79")]
    [InlineData(10_000L, "$0.01")]
    [InlineData(4_120L, "$0.0041")]
    [InlineData(37L, "$0.00004")]
    // One significant digit, rounded first: a carry drops a decimal, as the
    // Mac's .significantDigits(1) does (review round 2, low 3).
    [InlineData(94L, "$0.00009")]
    [InlineData(95L, "$0.0001")]
    [InlineData(99L, "$0.0001")]
    [InlineData(9L, "$0.000009")]
    [InlineData(1L, "$0.000001")]
    public void DollarsAreTheMacs(long micros, string expected) =>
        Assert.Equal(expected, InsightsBenchmarkWords.Dollars(micros));

    // A failed read, an undecodable reply and no daemon all say the core's unknown word.
    [Fact]
    public async Task AFailedReadIsTheUnknownWordAndNoDaemonIsToo()
    {
        var failed = new InsightsBenchmarksModel((m, p) => Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":[]}")), W);
        await failed.LoadAsync();
        Assert.True(failed.Failed);
        Assert.Equal(W("analytics_unavailable"), failed.Note);
        Assert.Empty(failed.Cards);

        var none = new InsightsBenchmarksModel((Func<string, string, Task<DaemonResponse>>?)null, W);
        await none.LoadAsync();
        Assert.Equal(W("analytics_unavailable"), none.Note);

        var empty = new InsightsBenchmarksModel((m, p) => Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":{\"enabled\":true,\"categories\":[]}}")), W);
        await empty.LoadAsync();
        Assert.Equal("Waiting for more data", empty.Note);
    }

    // A sample window's Insights never reaches the local store: every read
    // is refused, and Benchmarks is the Mac's sample, read before any tab.
    [Fact]
    public async Task ASampleWindowReadsNothingFromThisMachine()
    {
        var service = new SampleInsights();
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.CallAsync(new { operation = "week_overview" }, default));
        using var model = new InsightsViewModel(service, word => new InsightsBenchmarksModel(InsightsSample.Benchmarks, word));
        await model.SelectTabAsync(InsightsViewModel.BenchmarksTab);
        Assert.Equal(5, model.Benchmarks.Cards.Count);
        Assert.Equal("Fix a bug", model.Benchmarks.Cards[0].Name);
    }

    // The Mac's tab order: Overview, Breakdown, Benchmarks (InsightsView.swift:162).
    [Fact]
    public void BenchmarksIsTheThirdTab()
    {
        Assert.Equal((0, 1, 2), (InsightsViewModel.OverviewTab, InsightsViewModel.PatternsTab, InsightsViewModel.BenchmarksTab));
        Assert.Equal("Benchmarks", W("analytics_tab_benchmarks"));
    }

    // The pickers echo the chosen values whenever they rebind (each publish):
    // an echo never reads, even with a read in flight, and the option lists
    // are the same objects, so a rebind does not re-select.
    [Fact]
    public async Task APickerEchoDuringAReadAsksNothingMore()
    {
        var gate = new TaskCompletionSource<DaemonResponse>();
        int calls = 0;
        var model = new InsightsBenchmarksModel((m, p) => { calls++; return gate.Task; }, W);
        IReadOnlyList<BenchmarkChoice> periods = model.PeriodChoices;
        Task first = model.LoadAsync();
        Assert.True(model.Busy);
        // Not awaited: an echo that read again would wait on the same gate.
        Task echo = model.SelectAsync(4, "own");
        Assert.Equal(1, calls);
        Assert.True(echo.IsCompleted);
        Assert.Same(periods, model.PeriodChoices);
        gate.SetResult(DaemonResponse.Parse("{\"id\":1,\"result\":{\"enabled\":true,\"categories\":[]}}"));
        await first;
        await model.SelectAsync(4, "own");
        Assert.Equal(1, calls);
    }

    // The shared credential activity raises once per change.
    [Fact]
    public void TheCredentialActivityRaisesOnlyOnAChange()
    {
        var credential = new CredentialActivity();
        var seen = new List<bool>();
        credential.Changed += seen.Add;
        credential.Busy = true;
        credential.Busy = true;
        credential.Busy = false;
        Assert.Equal(new[] { true, false }, seen);
    }

    // ---- Review round 2, low 4: the three surviving mutants ----

    // The pickers' rows are indexes; the model maps them to the Mac's 4 and
    // 13 weeks (InsightsBenchmarksTab.swift:294, `[4, 13]`) and its
    // populations, so the request the period picker sends is pinned here.
    [Fact]
    public async Task ThePickersRowsAskForTheMacsPeriodsAndPopulations()
    {
        var asked = new List<string>();
        var model = new InsightsBenchmarksModel((method, parameters) =>
        {
            asked.Add(parameters);
            return Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":{\"enabled\":true,\"categories\":[]}}"));
        }, W);
        Assert.Equal(new[] { 4, 13 }, model.PeriodChoices.Select(c => c.Weeks));
        await model.ChooseAsync(periodIndex: 1, populationIndex: 0);
        await model.ChooseAsync(periodIndex: 0, populationIndex: 1);
        await model.ChooseAsync(periodIndex: -1, populationIndex: 0);
        await model.ChooseAsync(periodIndex: 2, populationIndex: 0);
        Assert.Equal(new[] { "{\"weeks\":13}", "{\"weeks\":4,\"population\":\"spawned\"}" }, asked);
        Assert.Equal(0, model.PeriodIndex);
        Assert.Equal(1, model.PopulationIndex);
    }

    // Yours: most traces first, then by name (InsightsBenchmarksTab.swift rows).
    [Fact]
    public void YoursListsMostTracesFirstThenByName()
    {
        var category = new BenchmarkCategory("feature", 12, new[]
        {
            new BenchmarkModel("b-model", "codex", 2, null, null, null),
            new BenchmarkModel("c-model", "claude_code", 7, null, null, null),
            new BenchmarkModel("a-model", "claude_code", 2, null, null, null),
        }, null, null, null, null);
        Assert.Equal(new[] { "c-model", "a-model", "b-model" }, InsightsBenchmarkWords.Rows(category, W).Select(r => r.Model));
    }

    // A reply without min_sessions waits for the daemon's own default, five
    // (InsightsBenchmarksTab.swift `answer.minSessions ?? 5`).
    [Fact]
    public void ACardWithNoPickWaitsForFiveTracesWhenTheReplyDoesNotSay()
    {
        var answer = new InsightsBenchmarks(true, true, 4, null, 2, null, new[]
        {
            new BenchmarkCategory("docs", 2, new[] { new BenchmarkModel("m", "codex", 2, null, null, null) }, null, null, null, null),
        });
        BenchmarkCard card = Assert.Single(InsightsBenchmarkWords.Cards(answer, W));
        Assert.Empty(card.Picks);
        Assert.Equal(InsightsBenchmarkWords.Fill(W("analytics_bench_compare_later"), new Dictionary<string, string> { ["n"] = "5" }), card.CompareLater);
    }
}
