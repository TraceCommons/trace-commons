using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Feed T's parts beyond the week itself, as the Mac draws them (product
/// owner, 2026-10-28 and 2026-10-29): spend at API prices (<c>work</c>),
/// Goals (<c>goals</c>), the NEAR AI card (<c>near_ai_credit</c>) and the
/// weekly summary card (the core's <c>comparisons</c> over <c>history</c>).
/// Ported from macos/Tests/TraceCommonsAppTests/InsightsWorkTests.swift:15-66,
/// InsightsCreditTests.swift:42-109 and InsightsModelTests.swift:720-985.
/// </summary>
public sealed class InsightsWeekExtrasTests
{
    private static readonly Lazy<Dictionary<string, string>> CoreCopy = new(() =>
    {
        var copy = new LocalInsights(Path.Combine(Path.GetTempPath(), "insights-copy-" + Guid.NewGuid().ToString("N")))
            .CallAsync(new { type = "copy" }, CancellationToken.None).GetAwaiter().GetResult();
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in copy.GetProperty("copy").EnumerateObject()) words[pair.Name] = pair.Value.GetString()!;
        return words;
    });

    private static string? Copy(string key) => CoreCopy.Value.TryGetValue(key, out string? word) ? word : null;

    private const string Dash = "—";

    private const string Base = """
        "enabled": true, "feed": "counter_pass", "readable": true, "iso_week": "2026-W38",
        "week_start": "2026-09-14", "overview": {"feed": "counter_pass"},
        "rollup": {"week_start": "2026-09-14", "coverage": {"known": 1, "partial": 0, "unknown": 0, "reasons": {}},
          "undated_sessions": 0, "sources": [], "by_model": [], "sessions": []}
        """;

    private const string History = """
        [{"week_start":"2026-09-14","comparable":false,"tokens":{"claude_code":37235},"cache_share_permille":{"claude_code":660},
          "patterns":{"repeated_reads":0,"retried_calls":0,"edit_fail_edit":0},"sessions":1,
          "pattern_counts":{"repeated_reads":0,"retried_calls":0,"edit_fail_edit":0,"long_context":0},
          "reread_files":0,"past_threshold":null}]
        """;

    private const string WorkJson = """
        {"basis":"api_list","week":{"sessions":2,"priced_sessions":1,"unpriced_sessions":1,"usd_micros_known":5,
         "usd_partial":true,"cache_savings_usd_micros_known":1,
         "edits_known":4,"edits_undone_known":1,"edits_denied_known":0,"edits_partial":false},
         "last_week":null,"by_tool":[],"by_model":[],"by_folder":[],"by_day":[{"date":"2026-10-23","usd_micros_known":5,
         "usd_partial":false}],"last_week_by_day":null}
        """;

    private const string GoalsJson = """
        {"goals":[{"tool":"claude_code","status":"open","met":false,"tokens":null,"time_secs":null,
                   "iterations":null,"folder":null,"session_ref":"t0.7","updated_at":"2026-09-14T09:09:00Z"},
                  {"tool":"codex","status":"complete","met":true,"tokens":42000,"time_secs":600,
                   "iterations":null,"folder":"web","session_ref":"t1.7","updated_at":"2026-09-14T09:00:00Z"},
                  {"tool":"codex","status":"from_a_newer_codex","met":false,"tokens":1,"time_secs":1,
                   "iterations":null,"folder":null,"session_ref":"t2.7","updated_at":"2026-09-14T08:00:00Z"}],
         "met":1,"total":3,"by_tool":{"claude_code":{"met":0,"total":1},"codex":{"met":1,"total":2}}}
        """;

    private static InsightsCounterWeek? Week(string extra) =>
        InsightsCounterWeek.Parse(JsonDocument.Parse("{" + Base + (extra.Length > 0 ? "," + extra : "") + "}").RootElement);

    private static InsightsWorkTotals Totals(int sessions, long usd, int? priced = null, int unpriced = 0, bool partial = false,
        string? key = null) =>
        new(key, sessions, priced ?? sessions, unpriced, null, usd, partial, 0, 0, 0, 0, false);

    private static InsightsWork Work(InsightsWorkTotals week, InsightsWorkTotals? lastWeek = null,
        IReadOnlyList<InsightsWorkModel>? byModel = null, IReadOnlyList<InsightsWorkTotals>? byTool = null,
        IReadOnlyList<InsightsWorkDay>? byDay = null, IReadOnlyList<InsightsWorkDay>? lastWeekByDay = null) =>
        new("api_list", week, lastWeek, byTool ?? Array.Empty<InsightsWorkTotals>(), byModel ?? Array.Empty<InsightsWorkModel>(),
            Array.Empty<InsightsWorkTotals>(), byDay ?? Array.Empty<InsightsWorkDay>(), lastWeekByDay);

    // ---- The wire --------------------------------------------------------

    [Fact]
    public void AWeekFromBeforeThePartsHasNone()
    {
        InsightsCounterWeek week = Week("")!;
        Assert.True(week.ShowsCounterPass);
        Assert.Null(week.Work);
        Assert.Null(week.Goals);
        Assert.Null(week.NearAiCredit);
        Assert.Null(week.UsageByDay);
        Assert.Null(week.History);
        Assert.Null(week.RecapCardEnabled);
        Assert.Null(week.ChangeVsLastWeek);
    }

    // InsightsWorkTests.swift:54-66, with the other parts beside it.
    [Fact]
    public void ThePartsDecodeFromTheWire()
    {
        InsightsCounterWeek week = Week(
            "\"work\":" + WorkJson + ",\"goals\":" + GoalsJson + ",\"history\":" + History
            + ",\"recap_card_enabled\":true,\"change_vs_last_week\":[{\"source\":\"claude_code\",\"permille\":-120},"
            + "{\"source\":\"codex\",\"permille\":null,\"unavailable\":\"below_coverage_floor\"}]"
            + ",\"usage_by_day\":[{\"date\":\"2026-10-27\",\"claude_code\":1500,\"codex\":null}]"
            + ",\"near_ai_credit\":{\"basis\":\"near_ai_ledger\","
            + "\"week\":{\"from\":\"a\",\"to\":\"b\",\"usd_micros\":4120000,\"sessions\":1,\"calls\":3,\"calls_without_charge\":0,\"tokens\":1200000},"
            + "\"today\":{\"from\":\"a\",\"to\":\"b\",\"usd_micros\":0,\"sessions\":0,\"calls\":0,\"calls_without_charge\":0},"
            + "\"last_31_days\":{\"from\":\"a\",\"to\":\"b\",\"usd_micros\":null,\"usd_micros_known\":5,\"usd_partial\":true,\"sessions\":1,\"calls\":4,\"calls_without_charge\":1},"
            + "\"by_day\":[{\"from\":\"a\",\"to\":\"b\",\"date\":\"2026-10-22\",\"usd_micros\":100000,\"sessions\":1,\"calls\":1,\"calls_without_charge\":0}]}")!;

        InsightsWork work = week.Work!;
        Assert.Equal(3, work.Week.EditsKeptKnown);
        Assert.True(work.Week.UsdPartial);
        Assert.Equal("2026-10-23", work.ByDay[0].Date);
        Assert.Null(work.LastWeek);
        Assert.Equal(3, week.Goals!.Total);
        Assert.Equal(new[] { "t0.7", "t1.7", "t2.7" }, week.Goals.Goals.Select(g => g.SessionRef));
        Assert.Null(week.Goals.Goals[0].Tokens);
        Assert.Equal(600, week.Goals.Goals[1].TimeSecs);
        Assert.Equal(true, week.RecapCardEnabled);
        Assert.Equal(JsonValueKind.Array, week.History!.Value.ValueKind);
        Assert.Equal(new long?[] { -120, null }, week.ChangeVsLastWeek!.Select(c => c.Permille));
        Assert.Equal(1500, week.UsageByDay![0].ClaudeCode);
        Assert.Null(week.UsageByDay[0].Codex);
        InsightsNearAiCredit credit = week.NearAiCredit!;
        Assert.Equal(4_120_000, credit.Week.UsdMicros);
        Assert.Null(credit.Last31Days.UsdMicros);
        Assert.Equal(5, credit.Last31Days.UsdMicrosKnown);
        Assert.Equal("2026-10-22", credit.ByDay![0].Date);
    }

    /// <summary>
    /// The Mac decodes the whole week or none of it: a part that is there but
    /// malformed refuses the reply, so it is the saved week with the notice.
    /// </summary>
    [Theory]
    [InlineData("\"work\":{\"basis\":\"api_list\"}")]
    [InlineData("\"goals\":{\"goals\":[{\"tool\":\"codex\"}],\"met\":0,\"total\":1}")]
    [InlineData("\"near_ai_credit\":{\"basis\":\"near_ai_ledger\"}")]
    [InlineData("\"usage_by_day\":[{\"claude_code\":1}]")]
    [InlineData("\"history\":{}")]
    [InlineData("\"recap_card_enabled\":\"yes\"")]
    [InlineData("\"change_vs_last_week\":[{\"permille\":1}]")]
    public void AMalformedPartIsNoWeek(string part) => Assert.Null(Week(part));

    [Fact]
    public void NullPartsAreAbsentParts()
    {
        InsightsCounterWeek week = Week("\"work\":null,\"goals\":null,\"near_ai_credit\":null,\"usage_by_day\":null,\"history\":null")!;
        Assert.Null(week.Work);
        Assert.Null(week.Goals);
        Assert.Null(week.NearAiCredit);
    }

    // ---- Spend (InsightsWorkTests.swift:15-52) ----------------------------

    [Fact]
    public void SpendIsItsKnownSumToTheCent()
    {
        Assert.Equal("$63.40", InsightsWorkWords.Spend(Totals(2, 63_400_000), Copy));
        Assert.Equal("$5.00", InsightsWorkWords.Spend(Totals(2, 5_000_000, unpriced: 1, partial: true), Copy));
        Assert.Equal(Dash, InsightsWorkWords.Spend(Totals(1, 0, priced: 0, partial: true), Copy));
        Assert.Equal("$896.21", InsightsWorkWords.Spend(Totals(2, 896_212_500), Copy));
        Assert.Equal("$0.00004", InsightsWorkWords.Spend(Totals(1, 40), Copy));
        Assert.Equal(Dash, InsightsWorkWords.Spend(null, Copy));
    }

    [Fact]
    public void ChangeVsLastWeekNeedsLastWeeksFigure()
    {
        Assert.Equal("▲ 18% vs last week", InsightsWorkWords.Change(118, 100, false, Copy)?.Words);
        Assert.Equal("▼ 12% vs last week", InsightsWorkWords.Change(88, 100, false, Copy)?.Words);
        Assert.True(InsightsWorkWords.Change(118, 100, false, Copy)?.Up);
        Assert.Null(InsightsWorkWords.Change(5, null, false, Copy));
        Assert.Null(InsightsWorkWords.Change(5, 0, false, Copy));
        Assert.Null(InsightsWorkWords.Change(12, 30, true, Copy));
        Assert.Null(InsightsWorkWords.Change(100, 100, false, Copy));
        Assert.Equal("$40.00", InsightsWorkWords.Spend(Totals(1, 40_000_000, priced: 0, partial: true), Copy));
    }

    [Fact]
    public void ModelRowsShareTheKnownSpend()
    {
        InsightsWork work = Work(Totals(2, 4_000_000), byModel: new[]
        {
            new InsightsWorkModel("gpt-5.5", "codex", 1, 10, 3_000_000, false),
            new InsightsWorkModel("claude-opus", "claude_code", 1, 10, 1_000_000, false),
            new InsightsWorkModel("unpriced", "codex", 1, 10, 0, true),
        });
        IReadOnlyList<InsightsWorkWords.ModelRow> rows = InsightsWorkWords.ModelRows(work, Copy);
        Assert.Equal(new[] { "gpt-5.5", "claude-opus" }, rows.Select(r => r.Label));
        Assert.Equal(new[] { "75%", "25%" }, rows.Select(r => r.Percent));
        Assert.Equal("Codex", rows[0].Tool);
        Assert.Equal("$3.00", rows[0].Amount);
    }

    /// <summary>
    /// The tiles (<c>InsightsWeekTiles</c>): a tile with nothing known is left
    /// out; the spend tile's note is the change, else whose API prices.
    /// </summary>
    [Fact]
    public void TheTilesLeaveOutWhatIsNotKnown()
    {
        Assert.Empty(InsightsWorkWords.Tiles(null, Copy));
        Assert.Empty(InsightsWorkWords.Tiles(Work(Totals(1, 0, priced: 0, partial: true)), Copy));

        InsightsWork claudeOnly = Work(Totals(2, 4_000_000), lastWeek: Totals(2, 2_000_000),
            byModel: new[] { new InsightsWorkModel("claude-opus", "claude_code", 1, 10, 4_000_000, false) },
            byTool: new[] { Totals(2, 4_000_000, key: "claude_code"), Totals(0, 0, key: "codex") });
        IReadOnlyList<InsightsWorkWords.Tile> tiles = InsightsWorkWords.Tiles(claudeOnly, Copy);
        Assert.Equal(new[] { Copy("analytics_tile_spend"), Copy("analytics_tile_top_model") }, tiles.Select(t => t.Label));
        Assert.Equal("$4.00", tiles[0].Value);
        Assert.Equal("▲ 100% vs last week", tiles[0].Note);
        Assert.Equal("claude-opus", tiles[1].Value);
        Assert.Equal("100% of spend · Claude", tiles[1].Note);

        InsightsWork noLastWeek = claudeOnly with { LastWeek = null };
        Assert.Equal(Copy("analytics_tile_api_prices_claude"), InsightsWorkWords.Tiles(noLastWeek, Copy)[0].Note);
    }

    [Fact]
    public void ApiPricesNamesOnlyTheToolsUsed()
    {
        InsightsWork both = Work(Totals(2, 1), byTool: new[] { Totals(1, 1, key: "claude_code"), Totals(1, 0, key: "codex") });
        Assert.Equal(Copy("analytics_tile_api_prices"), InsightsWorkWords.ApiPrices(both, Copy));
        InsightsWork codex = Work(Totals(1, 1), byModel: new[] { new InsightsWorkModel("gpt", "codex", 1, 1, 1, false) });
        Assert.Equal(Copy("analytics_tile_api_prices_codex"), InsightsWorkWords.ApiPrices(codex, Copy));
    }

    [Fact]
    public void TrendsShowOnlyWithSpendOrAChangeAndListTheToolsTokenChanges()
    {
        InsightsWork empty = Work(Totals(1, 0), byDay: new[] { new InsightsWorkDay("2026-10-05", 0, false) });
        Assert.False(InsightsWorkWords.ShowsTrends(empty, null, Copy));
        var tokens = new[] { new InsightsWeekChange("claude_code", -120, null), new InsightsWeekChange("codex", null, "below_coverage_floor") };
        Assert.True(InsightsWorkWords.ShowsTrends(empty, tokens, Copy));
        IReadOnlyList<InsightsWorkWords.ChangeLine> changes = InsightsWorkWords.Changes(empty, tokens, Copy);
        Assert.Equal(new[] { "Claude · " + Copy("analytics_card_tokens") }, changes.Select(c => c.Label));
        Assert.Equal("▼ 12% vs last week", changes[0].Words);

        InsightsWork spent = Work(Totals(1, 5), lastWeek: Totals(1, 5),
            byDay: new[] { new InsightsWorkDay("2026-10-05", 18_400_000, false), new InsightsWorkDay("2026-10-06", 0, false) },
            lastWeekByDay: new[] { new InsightsWorkDay("2026-09-28", 16_100_000, false) });
        Assert.True(InsightsWorkWords.ShowsTrends(spent, null, Copy));
        IReadOnlyList<InsightsWorkWords.DayPair> days = InsightsWorkWords.Days(spent);
        Assert.Equal(new[] { 18.4, 0 }, days.Select(d => d.Week));
        Assert.Equal(new double?[] { 16.1, null }, days.Select(d => d.LastWeek));
        Assert.Equal("Mon", days[0].Label);
    }

    // ---- NEAR AI (InsightsCreditTests.swift:42-109) ----------------------

    private static InsightsCreditWindow Window(long? usd, int calls = 0, long? tokens = null, string? date = null,
        long? known = null, bool? partial = null, long? tokensKnown = null) =>
        new(date, usd, 0, calls, 0, tokens, known, partial, tokensKnown, null);

    private static InsightsNearAiCredit Credit(InsightsCreditWindow week, IReadOnlyList<InsightsCreditWindow>? byDay = null) =>
        new(week, Window(0), byDay, week);

    [Fact]
    public void TheNearAiCardShowsOnlyWithCallsAndItsUnknownDaysAreDashes()
    {
        Assert.False(InsightsUsageWords.ShowsNearAi(Credit(Window(0))));
        Assert.False(InsightsUsageWords.ShowsNearAi(null));
        // The last 31 days decide, not the week on screen.
        Assert.True(InsightsUsageWords.ShowsNearAi(new InsightsNearAiCredit(Window(0), Window(0), null, Window(5, 2))));
        Assert.False(InsightsUsageWords.ShowsNearAi(new InsightsNearAiCredit(Window(5, 2), Window(0), null, Window(0))));
        var days = Enumerable.Range(0, 7).Select(i => Window(i == 3 ? null : i * 100_000L, 1, date: "2026-10-" + (22 + i))).ToArray();
        InsightsNearAiCredit shown = Credit(Window(5, 1), days);
        Assert.True(InsightsUsageWords.ShowsNearAi(shown));
        IReadOnlyList<InsightsUsageWords.Day> bars = InsightsUsageWords.Days(shown, Copy);
        Assert.Equal(7, bars.Count);
        Assert.Equal("2026-10-22", bars[0].Id);
        Assert.Null(bars[3].Value);
        Assert.Equal(Dash, bars[3].Amount);
        Assert.Equal("$0.10", bars[1].Amount);
        Assert.Empty(InsightsUsageWords.Days(Credit(Window(0)), Copy));
    }

    [Fact]
    public void APartlyKnownFigureReadsItsKnownSumPlain()
    {
        InsightsCreditWindow owner = Window(null, 421, known: 5_215_300, partial: true, tokensKnown: 2_000_000);
        Assert.Equal("$5.22 · " + InsightsAnalyticsWords.Fill(Copy("analytics_usage_tokens")!, ("t", InsightsAnalyticsWords.Figure(Copy, 2_000_000))),
            InsightsCreditWords.SpendAndTokens(owner, Copy));
        Assert.Equal(Dash, InsightsCreditWords.Spend(null, null, Copy));
        Assert.Equal(Dash + " · " + Dash, InsightsCreditWords.SpendAndTokens(Window(null, 1), Copy));
        Assert.Equal("$0.00005", InsightsCreditWords.Spend(null, 50, Copy));
        InsightsCreditWindow day = Window(null, 2, date: "2026-10-26", known: 250_000, partial: true);
        IReadOnlyList<InsightsUsageWords.Day> bars = InsightsUsageWords.Days(new InsightsNearAiCredit(owner, owner, new[] { day }, owner), Copy);
        Assert.Equal(0.25, bars[0].Value);
        Assert.Equal("$0.25", bars[0].Amount);
        // A day known only in part, but not flagged partial, draws no bar.
        InsightsCreditWindow unflagged = Window(null, 2, date: "2026-10-26", known: 250_000);
        Assert.Null(InsightsUsageWords.Days(new InsightsNearAiCredit(owner, owner, new[] { unflagged }, owner), Copy)[0].Value);
    }

    [Fact]
    public void NearAiFiguresPairSpendWithTokens()
    {
        string tokens = InsightsAnalyticsWords.Fill(Copy("analytics_usage_tokens")!, ("t", InsightsAnalyticsWords.Figure(Copy, 1_200_000)));
        Assert.Equal("$4.12 · " + tokens, InsightsCreditWords.SpendAndTokens(Window(4_120_000, 3, 1_200_000), Copy));
    }

    // ---- Goals (InsightsModelTests.swift:897-935) ------------------------

    [Fact]
    public void GoalsAreTheDaemonsSlashGoalsInTheCoresWords()
    {
        InsightsWeekGoals goals = Week("\"goals\":" + GoalsJson)!.Goals!;
        Assert.Equal("Goals met 1 of 3", InsightsGoalsWords.MetOf(goals, Copy));
        Assert.Equal(new[] { "Claude", "Codex", "Codex" }, goals.Goals.Select(g => InsightsGoalsWords.Tool(g.Tool, Copy)));
        Assert.Equal(new[] { "In progress", "Met", "Status unavailable" }, goals.Goals.Select(g => InsightsGoalsWords.Status(g.Status, Copy)));
        var shared = new Dictionary<string, string>
        {
            ["met"] = "Met", ["complete"] = "Met",
            ["open"] = "In progress", ["not_met"] = "In progress", ["active"] = "In progress",
            ["paused"] = "Paused", ["blocked"] = "Blocked",
            ["usage_limited"] = "Limit reached", ["budget_limited"] = "Limit reached",
        };
        foreach ((string wire, string shown) in shared)
        {
            Assert.Equal(shown, InsightsGoalsWords.Status(wire, Copy));
        }

        Assert.Equal(Dash + " tokens · " + Dash, InsightsGoalsWords.Line(goals.Goals[0], Copy));
        Assert.Equal("42,000 tokens · 10m", InsightsGoalsWords.Line(goals.Goals[1], Copy));
        Assert.Equal("1h 5m", InsightsGoalsWords.Span(3_900, Copy));
        Assert.Equal("0m", InsightsGoalsWords.Span(0, Copy));
        Assert.Equal("web", goals.Goals[1].Folder);
    }

    [Fact]
    public void GoalsShowOnlyUnderTheCounterPassWithAGoal()
    {
        Assert.True(InsightsGoalsWords.Shows(Week("\"goals\":" + GoalsJson)));
        Assert.False(InsightsGoalsWords.Shows(Week("\"goals\":{\"goals\":[],\"met\":0,\"total\":0}")));
        Assert.False(InsightsGoalsWords.Shows(Week("")));
        Assert.False(InsightsGoalsWords.Shows(null));
    }

    // ---- The weekly summary card (InsightsModelTests.swift:944-965) -------

    private const string RecapJson = """
        {"week_start":"2026-09-28","week_end":"2026-10-04","sessions":6,
         "sources":[{"source":"claude_code","tokens":800000,"change_permille":-200},
                    {"source":"codex","tokens":10,"change_permille":null}],
         "items":[{"kind":"best_cache_week","source":"claude_code","permille":450,"previous_best_permille":300},
                  {"kind":"goal","id":"goal-1","goal":{"kind":"weekly_tokens_under","source":"claude_code","tokens":50000},"met":true},
                  {"kind":"pattern_up","pattern":"repeated_reads","up_permille":1500},
                  {"kind":"from_a_newer_core"}],
         "past_threshold":null}
        """;

    [Fact]
    public void TheSummaryCardIsTheCoresItemsFilled()
    {
        InsightsRecap recap = InsightsRecap.Parse(JsonDocument.Parse(RecapJson).RootElement)!;
        Assert.Equal("6 traces · 20% fewer tokens than last week", InsightsComparisonsWords.RecapChange(recap.Sources[0], recap.Sessions, Copy));
        Assert.Null(InsightsComparisonsWords.RecapChange(recap.Sources[1], recap.Sessions, Copy));
        Assert.Equal(new[] { "Best cache week yet: 45%. Previous best 30%." },
            recap.Items.Select(i => InsightsComparisonsWords.RecapItem(i, Copy)).Where(l => l is not null));
        Assert.Null(InsightsComparisonsWords.RecapThreshold(recap, Copy));
        Assert.StartsWith("Your week · ", InsightsComparisonsWords.RecapTitle(recap, Copy));
        InsightsRecap past = InsightsRecap.Parse(JsonDocument.Parse(RecapJson.Replace("\"past_threshold\":null",
            "\"past_threshold\":{\"threshold\":150000,\"sessions\":2}", StringComparison.Ordinal)).RootElement)!;
        Assert.Equal("2 traces passed your 150,000 threshold.", InsightsComparisonsWords.RecapThreshold(past, Copy));
    }

    /// <summary>The core's comparisons, answered in order; every request recorded.</summary>
    private sealed class Recorder : ILocalInsights
    {
        private readonly Queue<Func<JsonElement>> _answers = new();
        public List<JsonElement> Requests { get; } = new();

        public Recorder Then(string json)
        {
            _answers.Enqueue(() => JsonDocument.Parse(json).RootElement.Clone());
            return this;
        }

        public Recorder ThenFail()
        {
            _answers.Enqueue(() => throw new InvalidOperationException("insights-operation-failed"));
            return this;
        }

        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            Requests.Add(JsonSerializer.SerializeToElement(operation));
            return Task.FromResult(_answers.Count > 1 ? _answers.Dequeue()() : _answers.Peek()());
        }
    }

    private const string Compared = """
        {"type":"comparisons","comparisons":{"feed":"counter_pass","week_start":"2026-09-14",
         "lever":{"pick":null,"disabled":[],"unavailable":"below_coverage_floor"},
         "recap":{"week_start":"2026-09-07","week_end":"2026-09-13","sessions":6,"sources":[],"items":[],"past_threshold":null}}}
        """;

    private const string Opened = """
        {"type":"recap_opened","state":{"goals":[],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":"2026-09-07"}}
        """;

    private static JsonElement HistoryElement => JsonDocument.Parse(History).RootElement.Clone();

    // InsightsModelTests.swift:826-836.
    [Fact]
    public async Task FeedSComparesNothingAndAsksTheCoreNothing()
    {
        var recorder = new Recorder().Then(Compared);
        var model = new InsightsComparisonsModel(recorder, () => 0);
        await model.LoadAsync(null, "2026-10-05", recapCardEnabled: true);
        Assert.Null(model.OpenRecap());
        await model.Settled;
        Assert.Empty(recorder.Requests);
        Assert.Null(model.Recap);
    }

    // InsightsModelTests.swift:784-801 and :838-851.
    [Fact]
    public async Task TheWeeklyFiguresGoToTheCoreUnchanged()
    {
        var recorder = new Recorder().Then(Compared);
        var model = new InsightsComparisonsModel(recorder, () => 3600);
        await model.LoadAsync(HistoryElement, "2026-09-14", recapCardEnabled: false);
        JsonElement request = Assert.Single(recorder.Requests);
        Assert.Equal("comparisons", request.GetProperty("type").GetString());
        Assert.Equal("2026-09-14", request.GetProperty("week_start").GetString());
        Assert.Equal(3600, request.GetProperty("tz").GetInt32());
        Assert.False(request.GetProperty("recap_card_enabled").GetBoolean());
        Assert.True(System.Text.Json.Nodes.JsonNode.DeepEquals(
            System.Text.Json.Nodes.JsonNode.Parse(HistoryElement.GetRawText()),
            System.Text.Json.Nodes.JsonNode.Parse(request.GetProperty("counter_weeks").GetRawText())));
        Assert.Equal("2026-09-07", model.Recap!.WeekStart);
        Assert.False(model.Failed);
    }

    [Fact]
    public async Task AFailedReadNeverKeepsAnEarlierWeeksCard()
    {
        var recorder = new Recorder().Then(Compared).ThenFail();
        var model = new InsightsComparisonsModel(recorder, () => 0);
        await model.LoadAsync(HistoryElement, "2026-09-14", recapCardEnabled: true);
        Assert.NotNull(model.Recap);
        await model.LoadAsync(HistoryElement, "2026-09-21", recapCardEnabled: true);
        Assert.Null(model.Recap);
        Assert.True(model.Failed);
    }

    [Fact]
    public async Task AnAnswerOfAnotherTypeIsAFailure()
    {
        var recorder = new Recorder().Then("{\"type\":\"week_overview\",\"comparisons\":{}}");
        var model = new InsightsComparisonsModel(recorder, () => 0);
        await model.LoadAsync(HistoryElement, "2026-09-14", recapCardEnabled: true);
        Assert.Null(model.Recap);
        Assert.True(model.Failed);
    }

    [Fact]
    public async Task TheSavedFeedTakesTheCardAway()
    {
        var recorder = new Recorder().Then(Compared);
        var model = new InsightsComparisonsModel(recorder, () => 0);
        await model.LoadAsync(HistoryElement, "2026-09-14", recapCardEnabled: true);
        Assert.NotNull(model.Recap);
        await model.LoadAsync(null, null, recapCardEnabled: false);
        Assert.Null(model.Recap);
        Assert.Single(recorder.Requests);
    }

    // "Open recap": the card is not shown again, and the closed week is put on screen.
    [Fact]
    public async Task OpeningTheCardWritesItsWeekAndReadsAgain()
    {
        var recorder = new Recorder().Then(Compared).Then(Opened).Then(Compared.Replace(
            "\"recap\":{\"week_start\":\"2026-09-07\",\"week_end\":\"2026-09-13\",\"sessions\":6,\"sources\":[],\"items\":[],\"past_threshold\":null}",
            "\"recap\":null", StringComparison.Ordinal));
        var model = new InsightsComparisonsModel(recorder, () => 0);
        await model.LoadAsync(HistoryElement, "2026-09-14", recapCardEnabled: true);

        Assert.Equal("2026-09-07", model.OpenRecap());
        await model.Settled;

        Assert.Equal(new[] { "comparisons", "recap_opened", "comparisons" }, recorder.Requests.Select(r => r.GetProperty("type").GetString()));
        Assert.Equal("2026-09-07", recorder.Requests[1].GetProperty("week_start").GetString());
        Assert.Null(model.Recap);
    }

    // InsightsModelTests.swift:803-824: the real core, and no store is made.
    [Fact]
    public async Task TheRealCoreComparesTheDaemonsWeeksWithoutCreatingAStore()
    {
        string store = Path.Combine(Path.GetTempPath(), "insights-compare-" + Guid.NewGuid().ToString("N"));
        var model = new InsightsComparisonsModel(new LocalInsights(store), () => 0);
        await model.LoadAsync(HistoryElement, "2026-09-14", recapCardEnabled: true);
        Assert.False(model.Failed);
        Assert.Null(model.Recap);
        Assert.False(Directory.Exists(store));
    }

    // ---- The sample week -------------------------------------------------

    /// <summary>The Mac's sample <c>work</c> and <c>goals</c>, figure for figure (InsightsSample.swift:34-63, :150-167).</summary>
    [Fact]
    public void TheSampleWeekCarriesTheMacsSpendAndGoals()
    {
        InsightsCounterWeek week = InsightsCounterWeek.Parse(InsightsSample.Week)!;
        InsightsWork work = week.Work!;
        Assert.Equal(125_500_000, work.Week.UsdMicrosKnown);
        Assert.Equal(37, work.Week.Sessions);
        Assert.Equal(112_000_000, work.LastWeek!.UsdMicrosKnown);
        Assert.Equal(new[] { "claude-opus-5-5", "claude-sonnet-5-5", "gpt-5-codex", "claude-haiku-5-5" }, work.ByModel.Select(m => m.Key));
        Assert.Equal(new long[] { 18_400_000, 24_900_000, 27_300_000, 21_600_000, 16_200_000, 6_800_000, 10_300_000 },
            work.ByDay.Select(d => d.UsdMicrosKnown));
        Assert.Equal("2026-10-05", work.ByDay[0].Date);
        Assert.Equal("2026-09-28", work.LastWeekByDay![0].Date);
        Assert.Equal(6, work.ByFolder.Count);
        Assert.Equal(2, week.Goals!.Met);
        Assert.Equal(3, week.Goals.Total);
        Assert.Equal(new[] { "met", "complete", "not_met" }, week.Goals.Goals.Select(g => g.Status));
        Assert.Null(week.NearAiCredit);
        Assert.Equal(0, week.History!.Value.GetArrayLength());
        Assert.Equal(false, week.RecapCardEnabled);
        Assert.Equal("$125.50", InsightsWorkWords.Spend(work.Week, Copy));
    }

    // ---- The window (InsightsView.swift:270-290, :380-389) -----------------

    /// <summary>The store: the core's words, and the comparisons the recorder answers.</summary>
    private sealed class Store : ILocalInsights
    {
        private readonly Recorder _comparisons;

        public Store(Recorder comparisons) => _comparisons = comparisons;

        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            JsonElement request = JsonSerializer.SerializeToElement(operation);
            string type = request.GetProperty("type").GetString()!;
            if (type == "copy")
            {
                return Task.FromResult(JsonSerializer.SerializeToElement(new { type = "copy", copy = CoreCopy.Value }));
            }

            if (type is "comparisons" or "recap_opened")
            {
                return _comparisons.CallAsync(operation, cancellationToken);
            }

            if (type == "list")
            {
                return Task.FromResult(JsonDocument.Parse("{\"type\":\"list\",\"insights\":[]}").RootElement.Clone());
            }

            throw new InvalidOperationException("insights-operation-failed");
        }
    }

    /// <summary>The sample week, with kept figures and the card switched as asked.</summary>
    private static string CountedWeek(bool recapCardEnabled)
    {
        var week = System.Text.Json.Nodes.JsonNode.Parse(InsightsSample.Week.GetRawText())!.AsObject();
        week["history"] = System.Text.Json.Nodes.JsonNode.Parse(History);
        week["recap_card_enabled"] = recapCardEnabled;
        return week.ToJsonString();
    }

    private static async Task<(InsightsViewModel Model, Recorder Comparisons, List<string?> Asked)> OpenWindow(
        Func<string?, string> week, Func<Task>? turnOffRecap = null)
    {
        var comparisons = new Recorder().Then(Compared);
        var asked = new List<string?>();
        var model = new InsightsViewModel(new Store(comparisons), weekReader: iso =>
        {
            asked.Add(iso);
            return Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":" + week(iso) + "}"));
        }, turnOffRecap: turnOffRecap);
        await model.LoadAsync();
        await model.Comparisons.Settled;
        return (model, comparisons, asked);
    }

    [Fact]
    public async Task FeedTAsksTheCoreForTheCardWithTheDaemonsFigures()
    {
        var (model, comparisons, _) = await OpenWindow(_ => CountedWeek(recapCardEnabled: true));
        using var _model = model;
        JsonElement request = Assert.Single(comparisons.Requests);
        Assert.Equal("2026-10-05", request.GetProperty("week_start").GetString());
        Assert.True(request.GetProperty("recap_card_enabled").GetBoolean());
        Assert.Equal(1, request.GetProperty("counter_weeks").GetArrayLength());
        Assert.Equal("2026-09-07", model.Comparisons.Recap?.WeekStart);
    }

    [Fact]
    public async Task TheSavedFeedAsksNothingAndDrawsNoCard()
    {
        var (model, comparisons, _) = await OpenWindow(_ => "{\"enabled\":false,\"feed\":\"saved\"}");
        using var _model = model;
        Assert.False(model.CounterFeed);
        Assert.Empty(comparisons.Requests);
        Assert.Null(model.Comparisons.Recap);
    }

    [Fact]
    public async Task ALaterSavedWeekTakesTheCardAway()
    {
        bool counted = true;
        var (model, _, _) = await OpenWindow(_ => counted ? CountedWeek(true) : "{\"enabled\":true,\"feed\":\"counter_pass\",\"readable\":false}");
        using var _model = model;
        Assert.NotNull(model.Comparisons.Recap);
        counted = false;
        await model.LoadWeekAsync();
        Assert.Null(model.Comparisons.Recap);
    }

    [Fact]
    public async Task ViewWeekPutsTheClosedWeekOnScreen()
    {
        var (model, comparisons, asked) = await OpenWindow(_ => CountedWeek(true));
        using var _model = model;
        comparisons.Then(Opened).Then(Compared);

        await model.OpenRecapAsync();

        Assert.Equal(new string?[] { null, "2026-W37" }, asked);
        Assert.Contains(comparisons.Requests, r => r.GetProperty("type").GetString() == "recap_opened");
    }

    [Fact]
    public async Task HideTurnsTheCardOffAndReadsTheWeekAgain()
    {
        int turnedOff = 0;
        var (model, _, asked) = await OpenWindow(_ => CountedWeek(true), () => { turnedOff++; return Task.CompletedTask; });
        using var _model = model;
        Assert.True(model.CanHideRecap);

        await model.HideRecapAsync();

        Assert.Equal(1, turnedOff);
        Assert.Equal(new string?[] { null, null }, asked);
    }

    [Fact]
    public async Task WithNoWayToTurnItOffThereIsNoHide()
    {
        var (model, _, asked) = await OpenWindow(_ => CountedWeek(true));
        using var _model = model;
        Assert.False(model.CanHideRecap);
        await model.HideRecapAsync();
        Assert.Single(asked);
    }

    private static string Shell(string relative) => File.ReadAllText(
        Path.Combine(AppContext.BaseDirectory, "shell-source", relative + ".txt")).Replace("\r\n", "\n", StringComparison.Ordinal);

    /// <summary>Each part is drawn only behind the Mac's gate, and only under feed T.</summary>
    [Fact]
    public void TheViewDrawsEachPartBehindTheMacsGate()
    {
        string view = Shell("TraceCommons.App/Controls/InsightsView.WeekExtras.cs");
        Assert.Contains("InsightsCounterWeek? week = ViewModel.CounterFeed ? ViewModel.CounterWeek : null;", view);
        Assert.Contains("if (week is not null && ViewModel.Comparisons.Recap is { } recap)", view);
        Assert.Contains("InsightsWorkWords.Tiles(week?.Work, copy)", view);
        Assert.Contains("if (week?.NearAiCredit is { } credit && InsightsUsageWords.ShowsNearAi(credit))", view);
        Assert.Contains("if (InsightsWorkWords.ModelRows(work, copy).Count > 0)", view);
        Assert.Contains("if (InsightsWorkWords.ShowsTrends(work, week.ChangeVsLastWeek, copy))", view);
        Assert.Contains("if (InsightsGoalsWords.Shows(week))", view);
        Assert.Contains("if (ViewModel.CanHideRecap)", view);
        string xaml = Shell("TraceCommons.App/Controls/InsightsView.xaml");
        Assert.Contains("<StackPanel x:Name=\"OverviewExtras\"", xaml);
        Assert.Contains("<StackPanel x:Name=\"PatternsGoals\"", xaml);
        Assert.Contains("WireWeekExtras();", Shell("TraceCommons.App/Controls/InsightsView.xaml.cs"));
        Assert.Contains("new InsightsRecapSwitch((method, parameters) => _host.CallAsync(method, parameters)).SetAsync(false)",
            Shell("TraceCommons.App/MainWindow.xaml.cs"));
    }
}
